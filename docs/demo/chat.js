/* The conversation screen: the messages (yours on the right, Claude's on the left), the message box at the bottom,
   Messages at the left and Actions at the right. Python calls window.CO.*; the page tells Python
   (postMessage) what was typed, what was tapped, and where it leaves room for what Claude opens. */
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
  var mode = "chat";    // "content" while something Claude opened is showing between the sidebars
  var shown = { left: true, right: true, bottom: true };    // the panels (see "the panels' switches")
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
  window.addEventListener("resize", function () { fitPanel(); follow(); fitStrip(); });
  function latestYou() { var y = log.querySelectorAll(":scope > .you"); return y.length ? y[y.length - 1] : null; }
  function makeRoom() {
    var you = latestYou(), was = room.offsetHeight, need = 0;
    if (you && mode === "chat") {
      // Where things are laid out (offsets: not moved by a message's rising into place).
      var margin = parseFloat(getComputedStyle(you).scrollMarginTop) || 0, top = -margin;
      for (var e = you; e; e = e.offsetParent) top += e.offsetTop;              // the page scrolled to it
      var end = column.offsetTop + column.offsetHeight - was;                    // where the conversation ends
      need = Math.max(0, Math.ceil(top + page.clientHeight - end));
    }
    if (need !== was) room.style.height = need + "px";
  }
  function follow() {
    makeRoom();
    if (stick && mode === "chat" && Date.now() > pinning) page.scrollTop = page.scrollHeight;
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
  // Before anything is said, the conversation starts with Chat Computer's first words: what a first program
  // prints, hello, world (lower case, with its comma, as in Kernighan's C tutorial and his book with Ritchie).
  var HELLO = "hello, world";
  function greet() {
    if (log.children.length) return;
    var m = message({ role: "assistant", text: HELLO, ts: now() });
    m.classList.add("hello");
    log.appendChild(m);
    body.classList.add("has-log");
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

  // Where the finished part of Claude's words ends: after the last blank line that isn't inside a code block.
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
    if (mode === "content") fitStrip();
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
    if (!msg || !msg.parentNode || msg.classList.contains("hello") || !/\b(you|ai)\b/.test(msg.className)) return null;
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
    } else if (k === "edit") {
      input.value = text;
      fitInput();
      focusBox();
      try { input.setSelectionRange(text.length, text.length); } catch (e) {}
    }
  }
  var coarse = !!(window.matchMedia && window.matchMedia("(any-pointer: coarse)").matches);
  log.addEventListener("click", function (e) {
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
    var was = body.classList.contains("hearing");
    liveText.textContent = text || "";
    body.classList.toggle("hearing", !!text);
    if (was !== !!text) fitStrip();
  }

  // ------------------------------------------------------------------ something open between the sidebars
  // While a website, a video or a document is open, the conversation shrinks to its latest exchange (your latest
  // message and Claude's latest words) just above the message box, and the window lays the page over the space
  // above: this says where that space is ("hole"), whenever it changes.
  var lastHole = "";
  // Above the message box, your latest message and Chat Computer's latest, each in full, in the order they came
  // (while you're speaking: Chat Computer's latest, then what you're saying). Nothing else shows there: notes and
  // older messages between or after them are left out (.off-strip), and the earlier conversation is above,
  // scrolled out of sight.
  function latest() {
    var yous = log.querySelectorAll(":scope > .you"), ais = log.querySelectorAll(":scope > .ai");
    var you = yous.length ? yous[yous.length - 1] : null, a = ais.length ? ais[ais.length - 1] : null;
    var rows = (body.classList.contains("hearing") ? [a] : [you, a]).filter(Boolean);
    rows.sort(function (x, y) { return x.compareDocumentPosition(y) & Node.DOCUMENT_POSITION_FOLLOWING ? -1 : 1; });
    var after = false;
    Array.prototype.forEach.call(log.children, function (n) {
      if (n === rows[0]) after = true;
      n.classList.toggle("off-strip", after && rows.indexOf(n) < 0);
    });
    return rows;
  }
  function fitStrip() {
    if (mode !== "content") return;
    var rem = remPx(), items = latest(), hearing = body.classList.contains("hearing");
    var first = items[0], h = first || hearing ? 2.5 * rem : 0;     // nothing said yet: the page reaches down
    if (first) {
      var end = hearing ? live : items[items.length - 1];
      h = Math.max(h, end.getBoundingClientRect().bottom - first.getBoundingClientRect().top + 0.25 * rem);
    }
    // Both in full, however many lines; past six-tenths of the screen they scroll, showing the end.
    rootStyle.setProperty("--strip-h", Math.round(Math.min(h, window.innerHeight * 0.6)) + "px");
    column.scrollTop = column.scrollHeight;
    reportHole();
  }
  // The frame: between the sidebars' cards (with the same space as around them), from their top down to the
  // latest exchange. The page goes inside it, clear of its rounded corners.
  function reportHole() {
    if (mode !== "content") return;
    var rem = remPx(), gap = Math.round(0.75 * rem), panels = body.classList.contains("panel");
    // With the sidebars: from half a line inside where each one's space ends (its card, or the narrow margin with
    // its switch when it's switched off), which is the same space as around the cards.
    var cs = getComputedStyle(body), half = Math.round(0.5 * rem);
    var left = panels ? Math.round(parseFloat(cs.paddingLeft) || 0) + half : gap;
    var right = panels ? window.innerWidth - Math.round(parseFloat(cs.paddingRight) || 0) - half : window.innerWidth - gap;
    var bottom = panels && !shown.bottom ? Math.round($("toggle-bottom").getBoundingClientRect().top) - gap
      : Math.round(column.getBoundingClientRect().top - 0.6 * rem);
    // "/" searching: its list goes in front of what's open. Where the window lays what's open over this page (on
    // a Chat Computer), nothing of the page can show over it, so it ends above the list instead.
    var list = $("results").getBoundingClientRect();
    if (searching && !openInPage && list.height) bottom = Math.min(bottom, Math.round(list.top) - gap);
    var f = $("frame").style;
    f.left = left + "px"; f.top = gap + "px";
    f.width = Math.max(0, right - left) + "px"; f.height = Math.max(0, bottom - gap) + "px";
    var inset = 2 + Math.round(0.4 * rem);              // the border, and room for the corners' curve
    var r = { x: left + inset, y: gap + inset, w: Math.max(0, right - left - 2 * inset), h: Math.max(0, bottom - gap - 2 * inset) };
    var key = r.x + "," + r.y + "," + r.w + "," + r.h;
    if (key !== lastHole) { lastHole = key; post({ type: "hole", x: r.x, y: r.y, w: r.w, h: r.h }); }
  }
  function setMode(m) {
    mode = m;
    body.classList.toggle("content", m === "content");
    if (m === "content") fitStrip();
    else {
      lastHole = "";
      Array.prototype.forEach.call(log.querySelectorAll(":scope > .off-strip"), function (n) {
        n.classList.remove("off-strip");
      });
      jumpDown();
    }
  }
  // Whether what's open is drawn in this same page (the browser demo's frame), so the page's own "/" list can go
  // over it; on a Chat Computer the window lays it over the page.
  var openInPage = false;

  // ------------------------------------------------------------------ the message box
  // A real text box, like the Claude app's: the cursor, arrow keys, selecting, pasting and input methods all work as
  // anywhere else. Enter sends; Shift+Enter starts a new line; Escape clears it. Page Up and Down scroll the
  // conversation, and so do the arrow keys, Home and End while the box is empty. It grows with what's typed.
  var drafting = false;
  function fitInput() {
    input.style.height = "auto";
    input.style.height = input.scrollHeight + "px";
    body.classList.toggle("drafting", input.value.trim() !== "");
    body.classList.toggle("typed", input.value !== "");          // the hint goes
    checkSearch();
    keysToPage();
  }
  // Whether Escape is the page's (to clear the box, or a sign-in form), rather than the window's (to close
  // what's open).
  function keysToPage() {
    var d = input.value.trim() !== "" || body.classList.contains("sheet");
    if (d !== drafting) { drafting = d; post({ type: "drafting", on: d }); }
  }
  function sendNow() {
    if (searching) { openResult(); return; }        // "/": Enter opens what's chosen
    var text = input.value.trim();
    if (!text) return;
    input.value = "";
    fitInput();
    post({ type: "send", text: text });
    stick = true;                    // it goes to the top of the screen when it's shown (CO.user)
  }
  var SCROLL_ALWAYS = { PageUp: "page_up", PageDown: "page_down" };
  var SCROLL_EMPTY = { ArrowUp: "up", ArrowDown: "down", Home: "top", End: "bottom" };
  input.addEventListener("input", fitInput);
  input.addEventListener("keydown", function (e) {
    if (e.isComposing || e.keyCode === 229) return;        // an input method is still putting a character together
    var k = e.key;
    if (searchKeys(e)) return;
    if (k === "Enter" && !e.shiftKey && !e.ctrlKey && !e.altKey && !e.metaKey) { e.preventDefault(); sendNow(); return; }
    if (k === "Escape" && body.classList.contains("show-messages")) { e.preventDefault(); hideMessages(); return; }
    if (k === "Escape" && input.value) { e.preventDefault(); input.value = ""; fitInput(); return; }
    var how = SCROLL_ALWAYS[k] || (input.value === "" ? SCROLL_EMPTY[k] : null);
    if (how) { e.preventDefault(); window.CO.scroll(how); return; }
    if (k.length === 1 && !e.ctrlKey && !e.altKey && !e.metaKey) measure(e.timeStamp);
  });
  $("send").addEventListener("click", sendNow);
  // Typing always goes into the box, even after a click elsewhere on the page or selecting words to copy them (but
  // not while a sign-in form is open: that has boxes of its own).
  function focusBox() {
    if (locked) { lockFocus(); return; }
    if (!body.classList.contains("sheet") && document.activeElement !== input) input.focus({ preventScroll: true });
  }
  document.addEventListener("keydown", function (e) {
    if (locked) { if (e.target !== $("lock-pin")) lockFocus(); return; }   // keys are for the PIN, nothing else
    if (body.classList.contains("sheet")) return;
    var paste = (e.ctrlKey || e.metaKey) && (e.key === "v" || e.key === "V");
    if (!(paste || (!e.ctrlKey && !e.altKey && !e.metaKey && (e.key.length === 1 || e.key === "Backspace" || e.key === "Enter")))) return;
    if (hiddenBottom()) setPanel("bottom", true);     // typing with the conversation switched off: it comes back
    if (e.target !== input) focusBox();
  }, true);
  document.addEventListener("mouseup", function () {
    var sel = window.getSelection();
    if (!sel || sel.isCollapsed) focusBox();
  });
  window.addEventListener("focus", focusBox);
  // The space the box takes at the bottom (it grows with what's typed): the conversation ends above it.
  new ResizeObserver(function () {
    rootStyle.setProperty("--composer-h", composer.offsetHeight + "px");
    follow();
    fitStrip();
  }).observe(composer);
  // The empty box's hint: all of it ("Chat computer anything or type “/” to search") where the box is wide enough,
  // otherwise its first half.
  var hint = $("hint");
  function fitHint() {
    hint.style.maxWidth = input.offsetWidth + "px";
    hint.classList.remove("short");
    if (hint.scrollWidth > hint.clientWidth + 1) hint.classList.add("short");
  }
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
  // Shown when there's room for them beside the conversation, and for their words (most screens; not a narrow
  // window).
  function fitPanel() {
    var rem = remPx();
    var col = parseFloat(rootStyle.getPropertyValue("--co-column")) || 30 * rem;
    var w = window.innerWidth;
    body.classList.toggle("panel", w * 0.17 >= 11 * rem && w * 0.66 - 3 * rem >= 0.8 * col);
  }
  // The panels' switches: Messages (left), Actions (right) and, while something is open, the conversation under
  // it (bottom), each switched off and on again with its switch, or by asking Claude (the settings left_panel,
  // right_panel and bottom_panel). Chat Computer keeps the choice: the page tells it, and it tells the page at the
  // start.
  var PANEL_NAMES = { left: "Messages", right: "Actions", bottom: "the conversation" };
  function hiddenBottom() { return mode === "content" && !shown.bottom && body.classList.contains("panel"); }
  function applyPanels() {
    ["left", "right", "bottom"].forEach(function (k) {
      body.classList.toggle("hide-" + k, !shown[k]);
      var b = $("toggle-" + k), label = (shown[k] ? "Hide " : "Show ") + PANEL_NAMES[k];
      b.setAttribute("aria-pressed", String(shown[k]));
      b.setAttribute("aria-label", label);
      b.title = label;
    });
    follow();
    fitStrip();
  }
  function setPanel(k, on) {
    if (shown[k] === on) return;
    shown[k] = on;
    applyPanels();
    post({ type: "panels", left: shown.left, right: shown.right, bottom: shown.bottom });
    if (k === "bottom" && on) focusBox();
  }
  ["left", "right", "bottom"].forEach(function (k) {
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
        if (li) handler(li, !!(e.target.closest && e.target.closest(".pin")));
      });
    });
  }

  // Actions: oldest at the top, grouped by day; pinned ones above.
  var dones = [], doneById = {}, lastDay = "", labelsFor = "", DONE_MAX = 300;
  function doneItem(e) {
    var li = document.createElement("li");
    li.className = "item done-item" + (e.pinned ? " pinned" : "");
    li.dataset.id = e.id;
    li.dataset.prompt = e.prompt_id || "";
    li.dataset.ts = String(e.prompt_ts || e.ts);
    var pinnedDay = e.pinned && new Date(e.ts * 1000).toDateString() !== new Date().toDateString();
    li.appendChild(el("done-when", (pinnedDay ? dayLabel(e.ts) + " " : "") + clock(e.ts)));
    li.appendChild(el("done-what", e.text));
    li.appendChild(pinButton(e.pinned));
    return li;
  }
  function addDone(e) {
    if (e.pinned) return;
    var day = new Date(e.ts * 1000).toDateString();
    if (day !== lastDay) { doneList.appendChild(dayItem(e.ts)); lastDay = day; }
    doneList.appendChild(doneItem(e));
  }
  function drawDone() {
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
    doneList.scrollTop = doneList.scrollHeight;
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
  onSide([doneList, donePinned], function (li, pin) {
    if (pin) { togglePin("done", doneById, li.dataset.id); drawDone(); return; }
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
  var ICON = { email: "i-email", telegram: "i-telegram", whatsapp: "i-chat" };
  function msgItem(m) {
    var li = document.createElement("li");
    li.className = "item msg" + (m.unread ? " unread" : "") + (m.pinned ? " pinned" : "");
    li.dataset.id = m.id;
    var top = el("msg-top");
    top.innerHTML = '<svg viewBox="0 0 24 24" aria-hidden="true"><use href="#' + (ICON[m.source] || "i-social") + '"/></svg>';
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
  onSide([msgList, msgPinned], function (li, pin) {
    var m = msgs[li.dataset.id];
    if (!m) return;
    if (pin) togglePin("messages", msgs, m.id);
    else { m.unread = false; hideMessages(); post({ type: "open_message", id: m.id }); }
    drawMessages();
  });
  $("add-account").addEventListener("click", function () { post({ type: "add_account" }); });
  $("connect-first").addEventListener("click", function () { post({ type: "add_account" }); });

  // ------------------------------------------------------------------ connecting an account
  // A form Chat Computer shows (CO.accountForm): what's typed in it goes back to Chat Computer's own sign-in store, never to
  // Claude. Choices (which kind of account) are buttons.
  var form = null;
  function showForm(f) {
    form = f;
    body.classList.toggle("sheet", !!f);
    body.classList.remove("sheet-busy");
    if (!f) { keysToPage(); focusBox(); return; }
    $("sheet-title").textContent = f.title || "";
    var text = $("sheet-text");
    text.textContent = "";
    (f.text || []).forEach(function (p) { var e = document.createElement("p"); e.textContent = p; text.appendChild(e); });
    var fields = $("sheet-fields");
    fields.textContent = "";
    var firstInput = null;
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
    if (f.choices && f.choices.length) {
      var row = el("choices");
      f.choices.forEach(function (c) {
        var b = document.createElement("button");
        b.type = "button";
        b.textContent = c.label;
        b.addEventListener("click", function () { post({ type: "add_account", provider: c.value }); });
        row.appendChild(b);
      });
      fields.appendChild(row);
    }
    $("sheet-error").textContent = f.error || "";
    $("sheet-ok").textContent = f.ok || "Connect";
    $("sheet-ok").style.display = f.ok === "" ? "none" : "";
    if (f.busy) body.classList.add("sheet-busy");
    keysToPage();
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
  $("sheet").addEventListener("keydown", function (e) { if (e.key === "Escape") { e.preventDefault(); cancelForm(); } });

  // ------------------------------------------------------------------ the lock screen
  // CO.lock({error}) covers everything with the time and a keypad; the PIN typed (on the keypad, or a keyboard) goes
  // back to Chat Computer to check, never to Claude. CO.lock(null) takes it away.
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
  // one between the sidebars; the last line asks Claude to look instead. Escape (clearing the box) ends it.
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
      row.innerHTML = useIcon("i-file");
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
    ask.appendChild(el("res-text", q ? "Ask Claude to find “" + q + "”" : "Ask Claude to find something"));
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
    if (r) post({ type: "open_file", path: r.path });
    else post({ type: "send", text: "Find “" + q + "” on my computer." });
    jumpDown();
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
  // message box and moving moves where typing goes; a sidebar item swiped left shows Pin, as Mail shows its buttons.
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
      pinch = { d: spread(e.touches), size: remPx(), f: 1 };
      return;
    }
    if (e.touches.length !== 1) { if (pinch) endPinch(false); touch = null; return; }
    var t = e.touches[0], target = e.target;
    touch = { x: t.clientX, y: t.clientY, target: target, moved: false, held: false, horiz: null,
              item: target.closest ? target.closest(".side-list .item, .side-pinned .item") : null,
              msg: target.closest ? target.closest("#log > .you, #log > .ai") : null, caret: null };
    var inBox = typeBox(target);
    post({ type: "keyboard", show: !!inBox });
    if (!touch.item || !touch.item.classList.contains("swiped")) unswipe(null);
    if (body.classList.contains("show-messages") && !(target.closest && target.closest("#messages"))) hideMessages();
    holdTimer = setTimeout(function () { held(touch); }, HOLD_MS);
  }, { passive: true, capture: true });
  function held(tc) {
    if (!tc || tc.moved) return;
    tc.held = true;
    if (tc.target === input) {                    // holding the box: moving now moves where typing goes
      tc.caret = { pos: input.selectionStart, line: parseFloat(getComputedStyle(input).lineHeight) || 36,
                   ch: (parseFloat(getComputedStyle(input).fontSize) || 24) * 0.55 };
    } else if (tc.msg) {                          // holding a message: all of it selected (three fingers copy it)
      selectContents(tc.msg);
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
    var t = e.changedTouches[0], now = Date.now();
    var again = now - taps.t < TAP_MS && Math.hypot(t.clientX - taps.x, t.clientY - taps.y) < 24;
    taps = { n: again ? taps.n + 1 : 1, t: now, x: t.clientX, y: t.clientY };
    if (taps.n === 2 || taps.n === 3) {
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
  // Messages from the top of the screen (the iPad's Notification Center): on a screen too narrow for the sidebars
  // it comes over the conversation until a touch elsewhere; with them showing, it goes back to its newest.
  function hideMessages() { body.classList.remove("show-messages"); }
  function showMessages() {
    if (body.classList.contains("panel")) msgList.scrollTop = 0;
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
      fitStrip();
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
      fitStrip();
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
      fitStrip();
    },
    state: function (name) {
      ["listening", "sleeping", "offline", "thinking", "speaking", "muted", "resting"].forEach(function (s) {
        body.classList.toggle(s, s === name);
      });
    },
    // Whether the microphone is on. (Not shown: the screen is the conversation; Chat Computer says so when it can't
    // hear.)
    mic: function (on) {},
    // Messages pulled down from the top of the screen (the touchscreen's Notification Center).
    showMessages: showMessages,
    // "/": what on this computer matches (n says which search it answers: older answers are ignored).
    searchResults: function (n, q, items, note) { if (searching && n === searchN) drawResults(q || "", items, note); },
    // "content" while something Claude opened is showing between the sidebars, "chat" otherwise.
    mode: function (m) { setMode(m === "content" ? "content" : "chat"); },
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
      greet();                        // nothing said yet: Chat Computer's hello, world
      trim();
      jumpDown();
      fitStrip();
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
    // The person's name and the model: nothing on this page shows them (before anything is said, the word logo and
    // Chat Computer's "hello, world").
    settings: function () {},
    // Which panels show: {left, right, bottom} (Chat Computer keeps the choice; see "the panels' switches").
    panels: function (s) {
      ["left", "right", "bottom"].forEach(function (k) { if (s && k in s) shown[k] = !!s[k]; });
      applyPanels();
    },
    // Text size (1 rem) and column width for this screen (chatcomputer/ui/layout.py).
    layout: function (sizePx, columnPx) {
      rootStyle.setProperty("--co-size", sizePx + "px");
      rootStyle.setProperty("--co-column", columnPx + "px");
      fitPanel();
      fitInput();
      follow();
      fitStrip();
    },
    // Page Up and Down, and the arrow keys, Home and End while the box is empty: reading back through the
    // conversation.
    scroll: function (how) {
      var line = parseFloat(getComputedStyle(body).lineHeight) || 36;
      var by = { page_up: -0.85 * page.clientHeight, page_down: 0.85 * page.clientHeight, up: -3 * line, down: 3 * line }[how];
      if (how === "top") page.scrollTo({ top: 0, behavior: "smooth" });
      else if (how === "bottom") page.scrollTo({ top: page.scrollHeight, behavior: "smooth" });
      else if (by) page.scrollBy({ top: by, behavior: "smooth" });
    },
    clear: function () { log.innerHTML = ""; ai = {}; users = {}; greet(); trim(); jumpDown(); }
  };
  showLive("");
  greet();
  fitPanel();
  fitInput();
  tick();
  window.CO.settings("", "");
  focusBox();
  post({ type: "ready" });
})();
