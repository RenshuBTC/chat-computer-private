/* How you want Chat Computer: the app's first page, and where it's changed later ("/" in Chat Computer opens it,
   with ?change). Two ways; the app does each (src-tauri/src/ways.rs):
   - App: the conversation, right away.
   - Its own system: Chat Computer Setup, which comes with the app, started (on a PC, Chat Computer's own system, put
     next to Windows; on a Mac, in Terminal, only on M1, M2 and M3, and the page says plainly when this Mac can't).
   The app keeps the way chosen; started again, it opens on the conversation (the app) or on this page, at the own
   system's steps. The page asks the app with way_status and choose_way (window.__TAURI__, given only to the app's own
   pages). */
(function () {
  "use strict";
  var tauri = window.__TAURI__ && window.__TAURI__.core;
  function app(command, args) {
    if (!tauri || typeof tauri.invoke !== "function") return Promise.reject("This page works in the Chat Computer app.");
    return tauri.invoke(command, args || {});
  }
  function $(id) { return document.getElementById(id); }
  var body = document.body;
  var status = { way: null, setup: "", os: { can: false, why: "", system: "" } };
  var changing = /[?&]change(=|&|$)/.test(location.search);
  var PANELS = { os: "way-os" };
  var busy = false;

  // The two choices, or the own system's steps.
  function show(which) {
    $("choose").hidden = which !== "choose";
    Object.keys(PANELS).forEach(function (w) { $(PANELS[w]).hidden = which !== w; });
    window.scrollTo(0, 0);
  }
  // Each choice as it stands here: the one chosen now (when changing it), and the own system where this computer
  // can't have it (with why, in its place).
  function mark() {
    Array.prototype.forEach.call(document.querySelectorAll(".way"), function (b) {
      var w = b.getAttribute("data-way"), cannot = w === "os" && !status.os.can;
      b.classList.toggle("now", changing && w === status.way);
      b.classList.toggle("cannot", cannot);
      b.setAttribute("aria-disabled", cannot ? "true" : "false");
    });
    var sys = status.os.system;
    $("os-what").textContent = !status.os.can && status.os.why ? status.os.why :
      sys === "mac" ? "Chat Computer in place of macOS when the Mac starts, on a Mac with M1, M2 or M3. macOS, your apps and " +
        "your files stay, one restart away." :
      sys === "windows" ? "Chat Computer in place of Windows when the PC starts. Windows stays, one restart away." :
      $("os-what").textContent;
  }

  // Copied, as a click lets a page do (in the click itself): a hidden box's words, or the browser's clipboard.
  function copy(text) {
    var ok = false;
    try {
      var t = document.createElement("textarea");
      t.value = text;
      t.setAttribute("readonly", "");
      t.style.cssText = "position:fixed;top:0;left:0;opacity:0";
      body.appendChild(t);
      t.select();
      ok = document.execCommand("copy");
      t.remove();
    } catch (e) { ok = false; }
    if (!ok && navigator.clipboard && navigator.clipboard.writeText) {
      navigator.clipboard.writeText(text).then(function () {}, function () {});
      ok = true;
    }
    return ok;
  }

  function osSteps(done) {
    $("setup-folder").textContent = (done && done.folder) || status.setup;
    $("os-started").hidden = !done;
  }

  // A way chosen: the app does it, and keeps it; then the conversation (the app), or the way's steps.
  function choose(way, button) {
    if (busy) return Promise.resolve();
    if (way === "os" && !status.os.can) { $("error").textContent = status.os.why; return Promise.resolve(); }
    $("error").textContent = "";
    busy = true;
    if (button) { button.classList.add("busy"); button.disabled = true; }
    return app("choose_way", { way: way }).then(function (done) {
      status.way = way;
      if (way === "app") { location.href = "../index.html"; return; }
      if (way === "os") osSteps(done);
      show(way);
    }, function (e) {
      $("error").textContent = String(e);
      if ($("choose").hidden) show("choose");
    }).then(function () {
      busy = false;
      if (button) { button.classList.remove("busy"); button.disabled = false; }
    });
  }

  document.addEventListener("click", function (e) {
    var t = e.target.closest ? e.target.closest("button") : null;
    if (!t) return;
    var way = t.getAttribute("data-way"), act = t.getAttribute("data-act");
    if (way) { choose(way, t); return; }
    if (t.hasAttribute("data-copy") || t.hasAttribute("data-copy-of")) {
      var text = t.getAttribute("data-copy") || $(t.getAttribute("data-copy-of")).textContent;
      if (copy(text)) { t.classList.add("done"); setTimeout(function () { t.classList.remove("done"); }, 2000); }
      return;
    }
    if (act === "choose") { changing = true; mark(); show("choose"); return; }
    if (act === "start") { location.href = "../index.html"; return; }
    if (act === "os") { choose(act, t); return; }
    if (act === "setup-folder") {
      app("show_folder", { which: "setup" }).then(function () {}, function (err) {
        $("error").textContent = String(err);
      });
    }
  });

  // What the app says: how it is now, and what's here. Started on the own system, this page opens at its steps;
  // otherwise (the first start, or to change it), at the two.
  app("way_status").then(function (s) {
    status = s;
    status.os = s.os || { can: false, why: "", system: "" };
    if (status.os.system) body.classList.add(status.os.system);
    osSteps(null);
    mark();
    if (!changing && s.way === "os") show(s.way);
    else show("choose");
  }, function (e) {
    mark();
    show("choose");
    $("error").textContent = String(e);
  });
})();
