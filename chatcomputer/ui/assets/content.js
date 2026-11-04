/* Chat Computer page helper. Runs in its own isolated script world inside every page shown on
   screen, so websites can't see or tamper with it. It lets the model read pages, find things to
   click, type, scroll and control video. Nothing here sends data anywhere except to Chat Computer. */
(function () {
  "use strict";
  if (window.__CO) return;
  var refs = new Map();
  var nextRef = 1;
  var SEL = 'a[href],button,input:not([type=hidden]),textarea,select,summary,[role=button],[role=link],[role=tab],' +
    '[role=menuitem],[role=checkbox],[role=radio],[role=option],[role=switch],[role=combobox],[role=searchbox],' +
    '[role=textbox],[role=treeitem],[role=gridcell],[contenteditable=""],[contenteditable=true],[onclick],' +
    '[tabindex]:not([tabindex="-1"])';

  function post(obj) {
    try { window.webkit.messageHandlers.chatcomputerPage.postMessage(JSON.stringify(obj)); } catch (e) {}
  }
  function refFor(el) {
    if (!el.__coRef) { el.__coRef = nextRef++; refs.set(el.__coRef, new WeakRef(el)); }
    return el.__coRef;
  }
  function byRef(r) { var w = refs.get(+r); return w ? w.deref() : null; }
  function clean(s, n) { s = String(s || "").replace(/\s+/g, " ").trim(); return n && s.length > n ? s.slice(0, n - 1) + "…" : s; }
  function visible(el) {
    var r = el.getBoundingClientRect();
    if (r.width < 2 || r.height < 2) return false;
    var cs = getComputedStyle(el);
    return cs.visibility !== "hidden" && cs.display !== "none" && +cs.opacity !== 0;
  }
  function inView(el) {
    var r = el.getBoundingClientRect();
    return r.bottom > 0 && r.right > 0 && r.top < innerHeight && r.left < innerWidth;
  }
  function labelledBy(el) {
    var ids = (el.getAttribute("aria-labelledby") || "").split(/\s+/).filter(Boolean);
    return ids.map(function (id) { var t = document.getElementById(id); return t ? t.textContent : ""; }).join(" ");
  }
  function nameOf(el) {
    var n = el.getAttribute("aria-label") || labelledBy(el);
    if (!n && el.labels && el.labels.length) n = el.labels[0].textContent;
    if (!n) n = el.innerText || el.textContent;
    if (!n) n = el.getAttribute("alt") || el.getAttribute("title") || el.getAttribute("placeholder") || el.value || el.name;
    if (!n) { var img = el.querySelector && el.querySelector("img[alt]"); if (img) n = img.alt; }
    return clean(n, 90);
  }
  function roleOf(el) {
    var r = el.getAttribute("role");
    if (r) return r;
    var t = el.tagName.toLowerCase();
    if (t === "a") return "link";
    if (t === "input") return (el.type || "text") === "text" ? "textbox" : (el.type || "input");
    if (t === "textarea") return "textbox";
    if (t === "select") return "dropdown";
    if (el.isContentEditable) return "textbox";
    return t === "summary" ? "button" : t;
  }
  function all(root, out, limit) {
    var list = root.querySelectorAll(SEL);
    for (var i = 0; i < list.length && out.length < limit; i++) out.push(list[i]);
    var hosts = root.querySelectorAll("*");
    for (var j = 0; j < hosts.length && out.length < limit; j++) if (hosts[j].shadowRoot) all(hosts[j].shadowRoot, out, limit);
    return out;
  }
  function interactive() {
    var els = all(document, [], 4000).filter(visible);
    var front = [], back = [];
    els.forEach(function (el) { (inView(el) ? front : back).push(el); });
    return front.concat(back);
  }
  function matches(name, words) {
    var low = name.toLowerCase();
    return words.every(function (w) { return low.indexOf(w) >= 0; });
  }
  function scroller() {
    var se = document.scrollingElement || document.documentElement;
    if (se.scrollHeight > se.clientHeight + 4) return se;
    var el = document.elementFromPoint(innerWidth / 2, innerHeight / 2);
    while (el && el !== document.body) {
      var cs = getComputedStyle(el);
      if (/(auto|scroll)/.test(cs.overflowY) && el.scrollHeight > el.clientHeight + 4) return el;
      el = el.parentElement;
    }
    return se;
  }
  function scrollInfo() {
    var s = scroller();
    var max = s.scrollHeight - s.clientHeight;
    return { scrollable: max > 4, scrollPercent: max > 4 ? Math.round(100 * s.scrollTop / max) : 0 };
  }
  function bestMedia() {
    var ms = Array.prototype.slice.call(document.querySelectorAll("video, audio"));
    if (!ms.length) return null;
    var playing = ms.filter(function (m) { return !m.paused && !m.ended; });
    if (playing.length) ms = playing;
    ms.sort(function (a, b) {
      var ra = a.getBoundingClientRect(), rb = b.getBoundingClientRect();
      return rb.width * rb.height - ra.width * ra.height;
    });
    return ms[0];
  }
  function mediaState(m) {
    if (!m) return { found: false };
    return { found: true, playing: !m.paused && !m.ended, position: Math.round(m.currentTime || 0),
      duration: isFinite(m.duration) ? Math.round(m.duration) : null, volume: Math.round((m.volume || 0) * 100),
      muted: !!m.muted, speed: m.playbackRate, title: document.title };
  }
  function anyPlaying() {
    return Array.prototype.some.call(document.querySelectorAll("video, audio"), function (m) {
      return !m.paused && !m.ended && !m.muted && m.volume > 0;
    });
  }
  function setNativeValue(el, value) {
    var proto = el.tagName === "TEXTAREA" ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
    var desc = Object.getOwnPropertyDescriptor(proto, "value");
    if (desc && desc.set) desc.set.call(el, value); else el.value = value;
    el.dispatchEvent(new Event("input", { bubbles: true }));
    el.dispatchEvent(new Event("change", { bubbles: true }));
  }
  function findByText(text) {
    var words = clean(text).toLowerCase().split(" ").filter(Boolean);
    if (!words.length) return null;
    var els = interactive();
    var exact = els.filter(function (el) { return nameOf(el).toLowerCase() === words.join(" "); });
    if (exact.length) return exact[0];
    var partial = els.filter(function (el) { return matches(nameOf(el), words); });
    if (partial.length) return partial[0];
    // Any visible text on the page (headings, labels, tiles without roles).
    var walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
    var n;
    while ((n = walker.nextNode())) {
      if (n.nodeValue && matches(n.nodeValue, words) && n.parentElement && visible(n.parentElement)) {
        var p = n.parentElement;
        var clickable = p.closest(SEL);
        return clickable || p;
      }
    }
    return null;
  }

  // When the page last changed (elements added or removed, text changed): after a click, Chat Computer waits until it
  // has been still a moment (GtkScreen.settle) instead of a fixed pause. Changes to attributes (a progress bar's
  // width, a video's controls fading) don't count: some pages make them all the time.
  var lastChange = Date.now();
  try {
    new MutationObserver(function () { lastChange = Date.now(); })
      .observe(document.documentElement, { childList: true, subtree: true, characterData: true });
  } catch (e) {}

  window.__CO = {
    quietFor: function () { return Date.now() - lastChange; },
    elements: function (find) {
      var words = clean(find).toLowerCase().split(" ").filter(Boolean);
      var lines = [];
      var seen = new Set();
      interactive().forEach(function (el) {
        if (lines.length >= 160) return;
        var name = nameOf(el);
        if (words.length && !matches(name + " " + (el.getAttribute("href") || ""), words)) return;
        var role = roleOf(el);
        var key = role + "|" + name + "|" + (el.getAttribute("href") || "");
        if (name && seen.has(key)) return;
        seen.add(key);
        var line = "[" + refFor(el) + "] " + role + ' "' + name + '"';
        if (el.tagName === "A" && el.href) {
          try { var u = new URL(el.href); line += " → " + (u.host === location.host ? "" : u.host) + clean(u.pathname, 50); } catch (e) {}
        }
        if ((el.tagName === "INPUT" || el.tagName === "TEXTAREA") && el.type !== "password" && el.value) line += ' = "' + clean(el.value, 60) + '"';
        if (el.type === "password") line += " (password)";
        if (el.checked) line += " (checked)";
        if (el.disabled) line += " (disabled)";
        if (!inView(el)) line += " (scroll to see)";
        lines.push(line);
      });
      return { title: document.title, url: location.href, lines: lines };
    },
    text: function (max) {
      var t = (document.body && document.body.innerText) || "";
      t = t.replace(/[ \t]+\n/g, "\n").replace(/\n{3,}/g, "\n\n").trim();
      var info = scrollInfo();
      return { title: document.title, url: location.href, text: t.slice(0, max || 30000),
        truncated: t.length > (max || 30000), scrollable: info.scrollable, scrollPercent: info.scrollPercent };
    },
    locate: function (ref, text) {
      var el = ref != null ? byRef(ref) : findByText(text);
      if (!el) return { error: ref != null ? "That element is gone; read the page again." : "Couldn't find “" + text + "”." };
      el.scrollIntoView({ block: "center", inline: "center", behavior: "instant" });
      var r = el.getBoundingClientRect();
      return { ref: refFor(el), x: r.left, y: r.top, w: r.width, h: r.height, dpr: window.devicePixelRatio || 1, name: nameOf(el) };
    },
    jsClick: function (ref) {
      var el = byRef(ref);
      if (!el) return { error: "That element is gone." };
      ["pointerdown", "mousedown", "pointerup", "mouseup"].forEach(function (t) {
        el.dispatchEvent(new MouseEvent(t, { bubbles: true, cancelable: true, view: window }));
      });
      el.click();
      return { ok: true };
    },
    focus: function (ref, clearFirst) {
      var el = byRef(ref);
      if (!el) return { error: "That element is gone; read the page again." };
      el.scrollIntoView({ block: "center", behavior: "instant" });
      el.focus();
      if (el.type === "password") return { password: true };
      if (clearFirst) {
        if ("value" in el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA")) { el.select(); }
        else if (el.isContentEditable) { document.execCommand("selectAll", false, null); }
      }
      return { ok: true, password: false };
    },
    insertText: function (text, submit) {
      var el = document.activeElement;
      if (!el) return { error: "Nothing is selected to type into." };
      if (el.type === "password") return { error: "password" };
      if (el.tagName === "INPUT" || el.tagName === "TEXTAREA") {
        var start = el.selectionStart != null ? el.selectionStart : el.value.length;
        var end = el.selectionEnd != null ? el.selectionEnd : el.value.length;
        setNativeValue(el, el.value.slice(0, start) + text + el.value.slice(end));
      } else if (el.isContentEditable) {
        document.execCommand("insertText", false, text);
      }
      if (submit) {
        var ev = { key: "Enter", code: "Enter", keyCode: 13, which: 13, bubbles: true };
        el.dispatchEvent(new KeyboardEvent("keydown", ev));
        el.dispatchEvent(new KeyboardEvent("keyup", ev));
        if (el.form) { if (el.form.requestSubmit) el.form.requestSubmit(); else el.form.submit(); }
      }
      return { ok: true };
    },
    scroll: function (dir, amount, ref) {
      if (ref != null) {
        var el = byRef(ref);
        if (el) el.scrollIntoView({ block: "center", behavior: "instant" });
        return scrollInfo();
      }
      var s = scroller();
      var step = (innerHeight * 0.85) * (amount || 1);
      if (dir === "top") s.scrollTop = 0;
      else if (dir === "bottom") s.scrollTop = s.scrollHeight;
      else s.scrollTop += dir === "up" ? -step : step;
      return scrollInfo();
    },
    media: function (action, value) {
      var m = bestMedia();
      if (!m) return { found: false };
      var v = value == null ? null : +value;
      switch (action) {
        case "play": m.play(); break;
        case "pause": m.pause(); break;
        case "toggle": if (m.paused) m.play(); else m.pause(); break;
        case "seek": if (v != null) m.currentTime = Math.max(0, v); break;
        case "forward": m.currentTime += (v || 10); break;
        case "rewind": m.currentTime = Math.max(0, m.currentTime - (v || 10)); break;
        case "volume": if (v != null) { m.volume = Math.max(0, Math.min(1, v / 100)); m.muted = false; } break;
        case "mute": m.muted = true; break;
        case "unmute": m.muted = false; break;
        case "speed": if (v) m.playbackRate = Math.max(0.25, Math.min(4, v)); break;
        case "fullscreen": m.classList.add("__co_full"); ensureFullStyle(); break;
        case "exit_fullscreen": m.classList.remove("__co_full"); break;
      }
      return mediaState(m);
    },
    duck: function (on) {
      document.querySelectorAll("video, audio").forEach(function (m) {
        if (on) { if (m.__coVol == null) { m.__coVol = m.volume; m.volume = m.volume * 0.25; } }
        else if (m.__coVol != null) { m.volume = m.__coVol; m.__coVol = null; }
      });
      return { ok: true };
    },
    state: function () { var m = bestMedia(); return { media: mediaState(m), playing: anyPlaying(), title: document.title, url: location.href }; }
  };
  function ensureFullStyle() {
    if (document.getElementById("__co_full_style")) return;
    var st = document.createElement("style");
    st.id = "__co_full_style";
    st.textContent = "video.__co_full{position:fixed!important;inset:0!important;width:100vw!important;height:100vh!important;" +
      "z-index:2147483647!important;background:#000!important;object-fit:contain!important;max-width:none!important;max-height:none!important}";
    (document.head || document.documentElement).appendChild(st);
  }
  var last = null;
  function report() {
    var now = anyPlaying();
    if (now !== last) { last = now; post({ type: "media", playing: now, title: document.title }); }
  }
  ["play", "playing", "pause", "ended", "emptied", "volumechange"].forEach(function (t) {
    document.addEventListener(t, report, true);
  });
  window.addEventListener("pagehide", function () { if (last) post({ type: "media", playing: false }); });

  // The touchscreen: a finger on a box to type in brings up Chat Computer's onscreen keyboard, as on an iPad; when
  // typing leaves the boxes, it goes away.
  var touchedAt = 0;
  document.addEventListener("touchstart", function () { touchedAt = Date.now(); }, { capture: true, passive: true });
  function typesIn(el) {
    if (!el) return false;
    if (el.isContentEditable || el.tagName === "TEXTAREA") return true;
    return el.tagName === "INPUT" && !/^(button|checkbox|radio|range|color|file|image|reset|submit|hidden)$/i.test(el.type);
  }
  document.addEventListener("focusin", function (e) {
    if (typesIn(e.target) && Date.now() - touchedAt < 1500) post({ type: "keyboard", show: true });
  }, true);
  document.addEventListener("focusout", function (e) {
    if (!typesIn(e.target)) return;
    setTimeout(function () { if (!typesIn(document.activeElement)) post({ type: "keyboard", show: false }); }, 0);
  }, true);
})();
