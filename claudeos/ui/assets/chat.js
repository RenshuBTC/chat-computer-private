/* The conversation screen. Python calls window.CO.* to add words; nothing here is clickable. */
(function () {
  "use strict";
  var log = document.getElementById("log");
  var liveText = document.getElementById("live-text");
  var body = document.body;
  var MAX_ITEMS = 60;
  var ai = {};          // id -> {el, md, text, raf}

  // The column is anchored at the bottom. Older messages that no longer fit are hidden whole,
  // so a line is never cut in half at the top of the screen.
  var fitRaf = 0;
  function fit() {
    fitRaf = 0;
    var kids = log.children, n = kids.length, i;
    var room = parseFloat(getComputedStyle(body).fontSize || "16") * 1.5;
    for (i = 0; i < n; i++) kids[i].classList.remove("gone");
    for (i = 0; i < n - 1; i++) {
      if (kids[i].getBoundingClientRect().top < room) kids[i].classList.add("gone"); else break;
    }
  }
  function scheduleFit() { if (!fitRaf) fitRaf = requestAnimationFrame(fit); }
  window.addEventListener("resize", scheduleFit);
  function trim() {
    while (log.children.length > MAX_ITEMS) log.removeChild(log.firstChild);
    body.classList.toggle("has-log", log.children.length > 0);
    scheduleFit();
  }
  function el(cls, text) {
    var d = document.createElement("div");
    d.className = cls;
    if (text != null) d.textContent = text;
    return d;
  }
  function ensureAI(id) {
    var m = ai[id];
    if (m) return m;
    var wrap = el("ai thinking");
    var md = el("md");
    wrap.appendChild(md);
    log.appendChild(wrap);
    trim();
    m = ai[id] = { el: wrap, md: md, text: "", raf: 0 };
    return m;
  }
  function paint(m) {
    m.raf = 0;
    m.md.innerHTML = window.renderMarkdown(m.text);
    if (m.text) m.el.classList.remove("thinking");
    scheduleFit();
  }

  var users = {};
  window.CO = {
    user: function (id, text, final) {
      liveText.textContent = "";
      body.classList.remove("hearing");
      var u = users[id];
      if (!u) { u = users[id] = el("you", text); log.appendChild(u); trim(); }
      else u.textContent = text;
      scheduleFit();
    },
    partial: function (text) {
      liveText.textContent = text || "";
      body.classList.toggle("hearing", !!text);
      scheduleFit();
    },
    ai: function (id, delta) {
      var m = ensureAI(id);
      m.text += delta;
      if (!m.raf) m.raf = requestAnimationFrame(function () { paint(m); });
    },
    aiStart: function (id) { ensureAI(id); },
    aiDone: function (id) {
      var m = ai[id];
      if (!m) return;
      if (m.raf) cancelAnimationFrame(m.raf);
      paint(m);
      m.el.classList.remove("thinking");
      if (!m.text && !m.el.querySelector(".activity")) { m.el.remove(); trim(); }
      delete ai[id];
    },
    activity: function (id, text) {
      var m = ensureAI(id);
      if (m.raf) { cancelAnimationFrame(m.raf); paint(m); }
      var a = el("activity", text);
      // Activity lines sit above the words they lead to.
      if (m.text) { m.el.appendChild(a); var md2 = el("md"); m.el.appendChild(md2); m.md = md2; m.text = ""; }
      else m.el.insertBefore(a, m.md);
    },
    notice: function (text, kind) {
      var n = el("notice" + (kind === "warn" ? " warn" : ""), text);
      log.appendChild(n);
      trim();
    },
    state: function (name) {
      ["listening", "sleeping", "offline", "thinking", "speaking", "muted"].forEach(function (s) {
        body.classList.toggle(s, s === name || (name === "offline" && s === "listening" && false));
      });
      if (name === "offline") body.classList.add("offline");
    },
    history: function (items) {
      log.innerHTML = "";
      (items || []).forEach(function (it) {
        if (it.role === "user") log.appendChild(el("you", it.text));
        else if (it.role === "assistant") {
          var w = el("ai"); var md = el("md"); md.innerHTML = window.renderMarkdown(it.text); w.appendChild(md); log.appendChild(w);
        }
      });
      trim();
    },
    settings: function (theme, name) {
      document.documentElement.setAttribute("data-theme", theme === "dark" ? "dark" : "light");
      var h = new Date().getHours();
      var part = h < 5 ? "Hello" : h < 12 ? "Good morning" : h < 18 ? "Good afternoon" : "Good evening";
      document.getElementById("greeting").textContent = part + (name ? ", " + name : "") + ".";
    },
    // Text size (1 rem) and column width for this screen (claudeos/ui/layout.py).
    layout: function (sizePx, columnPx) {
      var r = document.documentElement.style;
      r.setProperty("--co-size", sizePx + "px");
      r.setProperty("--co-column", columnPx + "px");
      scheduleFit();
    },
    clear: function () { log.innerHTML = ""; ai = {}; users = {}; trim(); }
  };
  window.CO.settings("light", "");
  try { window.webkit.messageHandlers.claudeos.postMessage(JSON.stringify({ type: "ready" })); } catch (e) {}
})();
