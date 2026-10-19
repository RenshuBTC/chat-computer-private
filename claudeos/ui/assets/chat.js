/* The conversation screen: the messages (yours on the right, Claude's on the left), the message box at the bottom,
   and What's done at the right. Python calls window.CO.*; the page tells Python (postMessage) what was typed and
   which item of What's done was tapped. */
(function () {
  "use strict";
  function $(id) { return document.getElementById(id); }
  var log = $("log"), liveText = $("live-text"), body = document.body;
  var input = $("input"), composer = $("composer"), doneList = $("done-list");
  var page = document.scrollingElement || document.documentElement;
  var rootStyle = document.documentElement.style;
  var MAX_ITEMS = 200;
  var ai = {};          // id -> {el, md, text, raf, cut, tail}
  function post(msg) {
    try { window.webkit.messageHandlers.claudeos.postMessage(JSON.stringify(msg)); } catch (e) {}
  }
  function now() { return Date.now() / 1000; }

  // Newest at the bottom, like a chat: while you're at the bottom, new words keep it there. Scrolled up to read
  // something earlier, it stays where you are until you scroll back down, say something or type.
  var stick = true;
  function atBottom() { return page.scrollHeight - page.scrollTop - page.clientHeight < 4; }
  window.addEventListener("scroll", function () { stick = atBottom(); }, { passive: true });
  window.addEventListener("resize", function () { fitPanel(); follow(); });
  function follow() { if (stick) page.scrollTop = page.scrollHeight; }
  function jumpDown() { stick = true; follow(); }

  // At most MAX_ITEMS messages on the page; the oldest go, but only while you're at the bottom (not while you
  // read back through earlier ones).
  function trim() {
    if (stick) while (log.children.length > MAX_ITEMS) log.removeChild(log.firstChild);
    body.classList.toggle("has-log", log.children.length > 0);
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
    var wrap = el("ai thinking");
    wrap.dataset.id = id;
    wrap.dataset.ts = String(now());
    var md = el("md");
    wrap.appendChild(md);
    log.appendChild(wrap);
    trim();
    m = ai[id] = { el: wrap, md: md, text: "", raf: 0, cut: 0, tail: [] };
    return m;
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
  function paint(m) {
    m.raf = 0;
    for (var i = 0; i < m.tail.length; i++) if (m.tail[i].parentNode === m.md) m.md.removeChild(m.tail[i]);
    m.tail = [];
    var cut = finishedUpTo(m.text, m.cut);
    if (cut > m.cut) { add(m.md, window.renderMarkdown(m.text.slice(m.cut, cut))); m.cut = cut; }
    var rest = m.text.slice(m.cut);
    if (rest) m.tail = add(m.md, window.renderMarkdown(rest));
    if (m.text) m.el.classList.remove("thinking");
    follow();
  }
  function showLive(text) {
    liveText.textContent = text || "";
    body.classList.toggle("hearing", !!text);
  }

  // ------------------------------------------------------------------ the message box
  // A real text box, like the Claude app's: the cursor, arrow keys, selecting, pasting and input methods all work as
  // anywhere else. Enter sends; Shift+Enter starts a new line; Escape clears it. Page Up and Down scroll the
  // conversation, and so do the arrow keys, Home and End while the box is empty. It grows with what's typed.
  function fitInput() {
    input.style.height = "auto";
    input.style.height = input.scrollHeight + "px";
    body.classList.toggle("drafting", input.value.trim() !== "");
  }
  function sendNow() {
    var text = input.value.trim();
    if (!text) return;
    input.value = "";
    fitInput();
    post({ type: "send", text: text });
    jumpDown();
  }
  var SCROLL_ALWAYS = { PageUp: "page_up", PageDown: "page_down" };
  var SCROLL_EMPTY = { ArrowUp: "up", ArrowDown: "down", Home: "top", End: "bottom" };
  input.addEventListener("input", fitInput);
  input.addEventListener("keydown", function (e) {
    if (e.isComposing || e.keyCode === 229) return;        // an input method is still putting a character together
    var k = e.key;
    if (k === "Enter" && !e.shiftKey && !e.ctrlKey && !e.altKey && !e.metaKey) { e.preventDefault(); sendNow(); return; }
    if (k === "Escape" && input.value) { e.preventDefault(); input.value = ""; fitInput(); return; }
    var how = SCROLL_ALWAYS[k] || (input.value === "" ? SCROLL_EMPTY[k] : null);
    if (how) { e.preventDefault(); window.CO.scroll(how); return; }
    if (k.length === 1 && !e.ctrlKey && !e.altKey && !e.metaKey) measure(e.timeStamp);
  });
  $("send").addEventListener("click", sendNow);
  // Typing always goes into the box, even after a click elsewhere on the page or selecting words to copy them.
  function focusBox() { if (document.activeElement !== input) input.focus({ preventScroll: true }); }
  document.addEventListener("keydown", function (e) {
    if (e.target === input) return;
    var paste = (e.ctrlKey || e.metaKey) && (e.key === "v" || e.key === "V");
    if (paste || (!e.ctrlKey && !e.altKey && !e.metaKey && (e.key.length === 1 || e.key === "Backspace" || e.key === "Enter"))) focusBox();
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
  }).observe(composer);

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

  // ------------------------------------------------------------------ the line under the box
  var STATUS = { sleeping: "Sleeping. Say “Claude”, press an AirPod or type to wake me.",
                 resting: "Sleeping. Say “Claude”, press an AirPod or type to wake me." };
  var micState = false;
  function status() {
    var s = "";
    for (var k in STATUS) if (body.classList.contains(k)) s = STATUS[k];
    if (!s && micState) s = "Listening";
    $("mic-text").textContent = s;
    $("mic").classList.toggle("on", !!s);
  }

  // ------------------------------------------------------------------ What's done
  var dones = [], lastDay = "", labelsFor = "", DONE_MAX = 300;
  function dayLabel(ts) {
    var d = new Date(ts * 1000), t = new Date(), y = new Date(t.getFullYear(), t.getMonth(), t.getDate() - 1);
    if (d.toDateString() === t.toDateString()) return "Today";
    if (d.toDateString() === y.toDateString()) return "Yesterday";
    return d.toLocaleDateString([], { weekday: "short", day: "numeric", month: "short" });
  }
  function addDone(e) {
    var day = new Date(e.ts * 1000).toDateString();
    if (day !== lastDay) {
      var h = document.createElement("li");
      h.className = "done-day";
      h.textContent = dayLabel(e.ts);
      doneList.appendChild(h);
      lastDay = day;
    }
    var li = document.createElement("li");
    li.className = "done-item";
    li.textContent = e.text;
    li.dataset.prompt = e.prompt_id || "";
    li.dataset.ts = String(e.prompt_ts || e.ts);
    li.title = new Date(e.ts * 1000).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
    doneList.appendChild(li);
  }
  function drawDone() {
    doneList.textContent = "";
    lastDay = "";
    dones.forEach(addDone);
    labelsFor = new Date().toDateString();
    body.classList.toggle("has-done", dones.length > 0);
    doneList.scrollTop = doneList.scrollHeight;
  }
  // Shown when there's room for it beside the conversation, and for its words (most screens; not a narrow window).
  function fitPanel() {
    var rem = parseFloat(getComputedStyle(document.documentElement).fontSize) || 24;
    var col = parseFloat(rootStyle.getPropertyValue("--co-column")) || 30 * rem;
    var w = window.innerWidth;
    body.classList.toggle("panel", w * 0.17 >= 11 * rem && w * 0.83 - 3 * rem >= 0.8 * col);
  }
  doneList.addEventListener("mousedown", function (e) { e.preventDefault(); });     // typing stays in the box
  doneList.addEventListener("click", function (e) {
    var li = e.target.closest ? e.target.closest(".done-item") : null;
    if (li) goTo(li.dataset.prompt, parseFloat(li.dataset.ts) || 0);
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

  var users = {};
  window.CO = {
    user: function (id, text, final) {
      showLive("");
      var u = users[id];
      if (!u) {
        u = users[id] = el("you", text);
        u.dataset.id = id;
        u.dataset.ts = String(now());
        log.appendChild(u);
        trim();
      } else u.textContent = text;
      jumpDown();
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
      delete ai[id];
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
        m.md = md2; m.text = ""; m.cut = 0; m.tail = [];
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
      status();
    },
    // Whether the microphone is on (ClaudeOS can hear you).
    mic: function (on) { micState = !!on; status(); },
    history: function (items) {
      log.innerHTML = "";
      (items || []).forEach(function (it) { log.appendChild(message(it)); });
      trim();
      jumpDown();
    },
    // Earlier messages put back above the conversation (from the transcript), then scrolled to the one asked for.
    earlier: function (items, id, ts) {
      var frag = document.createDocumentFragment(), day = "";
      (items || []).forEach(function (it) {
        var d = new Date(it.ts * 1000).toDateString();
        if (d !== day) { frag.appendChild(el("day", dayLabel(it.ts))); day = d; }
        frag.appendChild(message(it));
      });
      var first = firstTs();
      if (day && first && new Date(first * 1000).toDateString() !== day) frag.appendChild(el("day", dayLabel(first)));
      log.insertBefore(frag, log.firstChild);
      body.classList.toggle("has-log", log.children.length > 0);
      var target = findMessage(id, ts);
      if (target) reveal(target);
    },
    doneList: function (entries) { dones = (entries || []).slice(-DONE_MAX); drawDone(); },
    done: function (e) {
      var bottom = doneList.scrollHeight - doneList.scrollTop - doneList.clientHeight < 4;
      dones.push(e);
      if (dones.length > DONE_MAX || labelsFor !== new Date().toDateString()) {
        dones = dones.slice(-DONE_MAX);
        drawDone();                 // the oldest goes, or a new day began ("Today" is yesterday now)
      } else {
        addDone(e);
        body.classList.add("has-done");
      }
      if (bottom) doneList.scrollTop = doneList.scrollHeight;
    },
    settings: function (name, model) {
      var h = new Date().getHours();
      var part = h < 5 ? "Hello" : h < 12 ? "Good morning" : h < 18 ? "Good afternoon" : "Good evening";
      $("greeting").textContent = part + (name ? ", " + name : "") + ".";
      $("model").textContent = model || "";
    },
    // Text size (1 rem) and column width for this screen (claudeos/ui/layout.py).
    layout: function (sizePx, columnPx) {
      rootStyle.setProperty("--co-size", sizePx + "px");
      rootStyle.setProperty("--co-column", columnPx + "px");
      fitPanel();
      fitInput();
      follow();
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
    clear: function () { log.innerHTML = ""; ai = {}; users = {}; trim(); jumpDown(); }
  };
  showLive("");
  status();
  fitPanel();
  fitInput();
  window.CO.settings("", "");
  focusBox();
  post({ type: "ready" });
})();
