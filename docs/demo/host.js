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
   Fetched from elsewhere: a map from OpenStreetMap when you ask for one, the weather and the air from Open-Meteo,
   YouTube's player when connected Claude plays a video, and, when the browser's own AI looks something up for an
   answer (or you ask for the news), Wikipedia's articles and its Current events: only the words looked up go there.
   Google's own pages (its search, Google Maps' directions) and Wikipedia's show in the middle when asked for: of the
   sites that might be asked for, they're the ones that let another page show them. */
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
  // Earlier conversations, put back when something in Actions is tapped (the screen starts empty: the word logo).
  var EARLIER = [
    { role: "user", id: "y1", ts: START - 86400 - 3010, text: "Play some calm piano music" },
    { role: "assistant", id: "ya1", ts: START - 86400 - 3000, text: "Playing a calm piano mix on YouTube." },
    { role: "user", id: "y2", ts: START - 86400 - 2910, text: "A bit quieter please" },
    { role: "assistant", id: "ya2", ts: START - 86400 - 2900, text: "Volume is 40%." },
    { role: "user", id: "h1", ts: START - 5410, text: "How do I get to the airport on Sunday with two big suitcases?" },
    { role: "assistant", id: "ha1", ts: START - 5380, text: "The train means two changes, and one of them has stairs and no lift, so a taxi is easier: about 25 minutes. I put a packing list in your Documents too." }
  ];
  var DONE = [
    { id: "d1", ts: START - 86400 - 2995, text: "Opened youtube.com", prompt_id: "y1", prompt_ts: START - 86400 - 3010 },
    { id: "d2", ts: START - 86400 - 2905, text: "Set the volume to 40%", prompt_id: "y2", prompt_ts: START - 86400 - 2910 },
    { id: "d3", ts: START - 5400, text: "Searched the web for \u201ctrains to the airport on Sunday\u201d", prompt_id: "h1", prompt_ts: START - 5410 },
    { id: "d4", ts: START - 5390, text: "Saved Packing list.md", prompt_id: "h1", prompt_ts: START - 5410 }
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

  // ------------------------------------------------------------------ what's open in the middle
  // On Chat Computer a second browser view, laid over the frame the conversation page draws for it ("hole"); here an
  // iframe, a new one each time (a site can leave one blank for good). It's a page like any other: it scrolls, and
  // once clicked it has the keys (Google's search box, a map), until the message box is clicked. Its x closes it;
  // Escape makes the middle fill the screen (chat.js), here too while a page of the demo's own has the keys.
  var view = null, open = false, hole = null;
  var acted = 0;        // how many times what's open changed (shown, closed, the volume): see wordsOnly
  function frame() {
    if (view) view.remove();
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
    var v = frame();
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
  // A page's name (its <title>), for the top of its frame.
  function titleOf(html) {
    var m = /<title>([^<]*)<\/title>/i.exec(html || "");
    if (!m) return "";
    var t = document.createElement("textarea");
    t.innerHTML = m[1];
    return t.value;
  }
  function place() {
    if (!view || !open || !hole) return;
    view.style.left = hole.x + "px";
    view.style.top = hole.y + "px";
    view.style.width = hole.w + "px";
    view.style.height = hole.h + "px";
  }
  // Closed (its x, "close", a swipe up from the bottom edge): gone, and what played stops.
  function close() {
    if (!view) return;
    var was = open;
    acted++;
    open = false;
    player.token = {};
    player.id = "";
    view.remove();
    view = null;
    if (was) call("mode", "chat");
  }
  // Put away, for the conversation to have the middle again: out of sight, still playing (as on a Chat Computer, where
  // the conversation coming back leaves music playing). "Close" or "stop" then closes it.
  function putAway() {
    if (!open) return;
    open = false;
    view.style.display = "none";
    call("mode", "chat");
  }
  // A turn that did nothing to what's open (showed nothing, closed nothing, left the volume) and has words to say:
  // they'd be out of sight, the conversation being put away while something's open, so what's open is put away for
  // them. Only one thing has the middle at a time.
  function wordsOnly(since) { if (open && acted === since) putAway(); }
  function keys(e) {
    if (e.key === "Escape" && open && !e.isComposing) { e.preventDefault(); call("toggleFull"); }
  }
  // Page Up and Down (and the arrow keys, Home and End while the message box is empty) scroll what's open, when
  // it's a page of the demo's own (another site's page can't be reached from here: a click on it, and it scrolls).
  function scrollOpen(how) {
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
      "<p>The \u00d7 above closes it (so does a swipe up from the bottom edge); Escape makes it fill the screen.</p></article>",
      ".b{font:inherit;padding:0.4em 1.2em;border:2px solid var(--ink);border-radius:0.6em;background:var(--ground);color:var(--ink);cursor:pointer}",
      "var vol=" + player.volume / 100 + ";" + MUSIC) });
  }

  // ------------------------------------------------------------------ YouTube, when Claude plays a video
  // On Chat Computer, a video is YouTube's own page between the sidebars. Here, when connected Claude plays one,
  // it's YouTube's player in the same place, and the conversation comes back when the video ends. Browsers let a
  // page play sound by itself only after you've clicked, tapped or typed on the site (or, in Chrome, played its
  // videos with sound often enough): so a video starts with sound when the browser allows it and otherwise
  // without, and then the sound comes on with the first click, tap or key anywhere here, the video included. The
  // player's own controls pause it. (The player and this page talk with YouTube's player messages.)
  var player = { token: {}, id: "", heard: false, state: -2, armed: false, volume: 100 };
  function youtube(id, start, title) {
    start = start || 0;
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
  // Without Claude (Connect Claude) or Chrome's built-in AI (below), the page answers from what's written here: what
  // Chat Computer is and how to get it, what this page can show, and honest answers about what it can't do without an
  // AI. The facts are README.md's, in one place, for the written answers and for Chrome's AI.
  // The browser's own AI: Chrome's is Gemini Nano, from Google; Edge has the same, with Microsoft's Phi-4-mini.
  var EDGE = /\bEdg\//.test(navigator.userAgent || "");
  var AI = { browser: EDGE ? "Edge" : "Chrome", model: EDGE ? "Phi-4-mini" : "Gemini Nano", short: EDGE ? "Phi-4-mini" : "Gemini",
             maker: EDGE ? "Microsoft" : "Google" };
  var FACTS = {
    what: "Chat Computer is Claude, made by Anthropic, running a whole computer: there's no desktop and no apps to learn. " +
      "You talk, type or touch the screen, and it does what you ask: opens websites, plays music and videos, reads your " +
      "messages, looks after your files and settings.",
    soon: "Chat Computer isn't out yet: the download is coming soon.",
    free: "Chat Computer itself is free. It runs on your own Claude Pro or Max plan, which is where Claude comes from.",
    pc: "On a Windows PC it installs next to Windows, on free space: Windows, your apps and your files stay, one restart " +
      "away, and you can remove Chat Computer any time. The PC must start in UEFI mode (every PC from the last ten years " +
      "does) and have about 70 GB free.",
    mac: "On a Mac with Apple silicon it installs next to macOS the same way: M1 and M2 Macs work fully, M3 Macs with some " +
      "limits for now, and Intel Macs or M4 and newer can't run it yet. The Mac version is the newest: it hasn't been " +
      "tried on a real Mac yet.",
    use: "You can touch the screen, type on a keyboard or in the Claude app on your phone, or talk through AirPods.",
    private: "On Chat Computer your files stay on your computer, and speech is turned into text on the computer itself. " +
      "What you ask goes to Claude through your own Claude account, as in the Claude app.",
    indie: "Chat Computer is an independent project, not made by Anthropic. It runs on Claude, which Anthropic makes."
  };
  var CITIES = { singapore: [103.6, 1.2, 104.1, 1.47], london: [-0.35, 51.38, 0.15, 51.62], "new york": [-74.1, 40.6, -73.8, 40.85],
                 tokyo: [139.55, 35.55, 139.95, 35.8], paris: [2.22, 48.8, 2.47, 48.92], sydney: [151.0, -33.98, 151.35, -33.78] };
  var TRY = "Try “play some piano music”, “show me a map of Tokyo”, “weather in London”, " +
            "“set a timer for 5 minutes” or “what did Jane say?”. Type / to search the files.";
  var REAL = "Connect Claude, at the right of the message box, and Claude answers for real, with your own API key.";
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
  function weather(t) {
    var asked = placeIn(t), where = asked || hereName() || "Singapore", tomorrow = /\btomorrow\b/i.test(t);
    return { later: function () {
      return place_(where).then(function (p) {
        return fetch("https://api.open-meteo.com/v1/forecast?latitude=" + p.lat + "&longitude=" + p.lon +
          "&current=temperature_2m,weather_code&daily=weather_code,temperature_2m_max,temperature_2m_min," +
          "precipitation_probability_max&timezone=auto&forecast_days=2").then(function (r) { return r.json(); }).then(function (w) {
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
  // The browser's built-in AI runs on the device and can't reach the internet by itself, so this page looks things up
  // for it, from the browser, in sources that let a page ask them directly and need no key: Wikipedia (the opening of
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
    })).then(function (two) { return two[0].concat(two[1]); });
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

  var JOKES = ["I told my computer I needed a break. It said: “No problem, I'll go to sleep.”",
               "Why did the computer go to the doctor? It had a virus, and a terrible case of bytes.",
               "I'd tell you a joke about Wi-Fi, but I'm not sure you'd get the connection."];
  var OTHER = {
    zh: "你好！这是 Chat Computer 的屏幕：Claude 来操作整台电脑，你只要说话、打字或触摸屏幕，它就替你完成。这个页面的回答是预先写好的，而且目前只用英文。试试输入 “play some piano music”。",
    ja: "こんにちは！これは Chat Computer の画面です。Claude がコンピューター全体を動かし、話す・打つ・触るだけで何でもしてくれます。このページでは今のところ英語でお試しください。",
    es: "¡Hola! Esto es Chat Computer: Claude maneja todo el ordenador, y tú solo hablas, escribes o tocas la pantalla. " +
        "Por ahora esta página responde en inglés: prueba “play some piano music” o “show me a map of Tokyo”.",
    ms: "Hai! Ini Chat Computer: Claude menjalankan seluruh komputer, dan anda hanya bercakap, menaip atau menyentuh skrin. " +
        "Buat masa ini halaman ini menjawab dalam bahasa Inggeris: cuba “play some piano music”."
  };
  function language(t) {
    if (/[぀-ヿ]/.test(t)) return "ja";
    if (/[一-鿿]/.test(t)) return "zh";
    if (/[¿¡]|\b(hola|qué|que es|cómo|esto|puedes|gracias)\b/i.test(t)) return "es";
    if (/\b(apa (ini|itu|khabar)|boleh|awak|anda|terima kasih|saya)\b/i.test(t)) return "ms";
    return "";
  }
  // Asked to write, explain or translate something: only an AI can (Chrome's built-in one when there is one, else an
  // honest note, see fallback), whatever else the words mention ("write an email to my boss" isn't about Jane's).
  var OPEN_ENDED = /\b(write|draft|compose|poem|story|essay|explain|summari[sz]e|translate|how do you say|in (japanese|french|spanish|german|chinese|korean|italian)|recipe for|how (do i|to) (make|cook|bake)|plan (a|my)|itinerary|advice|tips|capital of|python|javascript)\b/i;
  // What was asked for, for the honest note: "I can't write a recipe on this page without an AI".
  var WANTS = [[/recipe|cook|bake/i, "write a recipe"], [/poem/i, "write a poem"], [/story/i, "write a story"],
               [/email|letter/i, "write an email"], [/python|javascript|code|function/i, "write code"],
               [/translate|how do you say|in (japanese|french|spanish|german|chinese|korean|italian)/i, "translate"],
               [/explain|summari/i, "explain that"], [/plan|itinerary|trip/i, "plan that"], [/advice|tips/i, "give advice"]];
  var INTENTS = [
    // Closing, and the volume, before music: "stop the music", "close the song" or "turn the music down" have music
    // words in them too (and would start it again).
    [/^(close|stop|pause)\b|\b(close|stop|pause|hide|turn off|switch off)\s+(the\s+|that\s+|this\s+|my\s+)?(it|that|this|music|videos?|page|songs?|piano|playing|youtube|timer|map)\b|\b(turn|switch)\s+(it|that|this|the\s+(music|video|song|piano))\s+off\b/i, function () {
      return { text: view ? "Closed it." : "Nothing's open.", act: close };
    }],
    [/\b(volume|louder|quieter|softer|mute|unmute)\b|\bturn\s+(up|down|(it|that|this|the\s+(music|sound|song|video|piano|volume))\s+(up|down))\b/i, function (t) {
      if (/\bmute\b/i.test(t)) return { text: "Muted.", done: ["Muted the sound"], act: function () { setVolume(0); } };
      var asked = /\b(\d{1,3})\s*(%|percent\b)?/i.exec(t);
      if (asked) {
        var p = Math.min(100, +asked[1]);
        return { text: "Volume is " + p + "%.", done: ["Set the volume to " + p + "%"], act: function () { setVolume(p); } };
      }
      var up = /\b(louder|up|unmute)\b/i.test(t);
      return { text: up ? "Volume is 60%." : "Volume is 40%.", done: [up ? "Turned the volume up" : "Turned the volume down"],
        act: function () { setVolume(up ? 60 : 40); } };
    }],
    // Who's answering, and who made it.
    [/\b(are you|is this|who are you|which (ai|model)|what (ai|model))\b.*\b(claude|chatgpt|gpt|gemini|ai|real|human|bot|model)\b|^(who are you|are you real)\b|\bwhat('s| is) (running|powering|behind) (you|this)\b|\bwho('s| is) (answering|talking|running this)\b/i, function () {
      return { text: (nano.state === "available" && nano.session ? "Right now " + AI.browser + "'s built-in AI (" + AI.model + ") is answering, on your " +
        "device, with what's written here about Chat Computer, and what it looks up on Wikipedia." : "On this page my answers are written in advance.") +
        " On a real Chat Computer, Claude answers, through your own Claude plan. " + REAL };
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
      return { text: "No, they're made up for this page. On a Chat Computer, Messages shows yours, from the email and chat accounts you connect." };
    }],
    [/\b(my (actual|real|own) (computer|files|pc)|on my (computer|pc|laptop)|control my (pc|computer))\b/i, function () {
      return { text: "Not from this page: a website can't reach your computer, and the files here are made up. Installed, " +
        "Chat Computer opens your own files, from your Windows folders too." };
    }],
    [/\bwhat (can'?t|can not|cannot) you do\b|\byour limits\b|\bwhat don'?t you do\b/i, function () {
      return { text: "On this page I can only show a few things: music the browser makes, maps, timers, the weather, the news, " +
        "Wikipedia's articles and the made-up messages and files. Real answers need Claude (Connect Claude) or the AI built into " +
        "Chrome and Edge. On a Chat Computer, Claude does " +
        "most of what a computer does, by talking; it doesn't run Windows or Mac apps (it uses websites instead), and " +
        "Windows stays one restart away." };
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
        "M1, M2 or M3; the internet; and a Claude Pro or Max plan. " + FACTS.use + " " + FACTS.soon };
    }],
    [/\b(privacy|private|my data|data|secure|security|safe to use|track|spy|listening to me)\b/i, function () {
      return { text: "On this page, what you type stays in your browser, unless you connect Claude (then it goes to Anthropic). " +
        "For a map or the weather only the place's name goes out (to Open-Meteo), and when the AI built into Chrome and Edge, which " +
        "runs on your device, looks something up or you ask for the news, only the words looked up go out (to Wikipedia). " +
        "Google's own pages show here when you open Google, ask for directions or look up something Wikipedia hasn't got: " +
        "those words go to Google, as they would on google.com. " +
        FACTS.private };
    }],
    [/\b(do i need|need|require|have to have)\b.*\b(airpods|headphones|earbuds|a keyboard|a mouse|a touch ?screen)\b/i, function () {
      return { text: "No. " + FACTS.use + " AirPods are for talking to it." };
    }],
    // What it is.
    [/\b(operating system|an os|is (this|it) an? (os|operating))\b/i, function () {
      return { text: "Yes, in a way: Chat Computer runs the whole computer, with Claude in charge instead of a desktop. Underneath, " +
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
        "Computer Setup and sign in to Claude, and it makes room and installs itself next to Windows, usually in well under an " +
        "hour; Windows stays. The PC must start in UEFI mode (any from the last ten years) and have about 70 GB free. " +
        FACTS.free + " " + FACTS.soon };
    }],
    [/^\s*(hi|hello|hey|hiya|yo|good (morning|afternoon|evening)|\u{1F44B}|help)( there| claude| chat ?computer| everyone| all)?[\s!.,?]*$|\bwhat can you do\b|\bhow does (this|it) work\b|\bwhat is (this|chat ?computer)\b|^what('?s| is) this\b|^\?+$/iu, function () {
      return { text: "Hi! I'm Chat Computer: Claude, running the whole computer. You talk, type or touch, and I do it: open websites, " +
        "play music, read your messages, look after your files and settings.\n\nOn this page my answers are written in advance " +
        "(or, in Chrome and Edge, come from their built-in AI), and what you type stays in your browser. " + REAL + " " + FACTS.indie + " " + TRY };
    }],
    [/^\s*(how are you|how'?s it going|how do you do)\b/i, function () { return { text: "I'm good, thanks! What would you like to try? " + TRY }; }],
    [/\b(useless|stupid|dumb|rubbish|sucks?|hate you)\b|\b(you'?re|you are|this is|that'?s|it'?s) (so )?(terrible|awful|the worst)\b/i, function () {
      return { text: "Sorry I'm not much help yet: on this page my answers are written in advance. " + REAL + " " + TRY };
    }],
    // Things it can show or do here. The air first ("where is the haze from?" isn't a map).
    [AIRY, function (t) { return { search: t, later: function () { return airAnswer(t); } }; }],
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
      var m = /(\d+)\s*(min|minute|m\b)/i.exec(t), eggs = /\beggs?\b/i.test(t), mins = m ? Math.min(180, +m[1]) : eggs ? 9 : 10;
      return { text: (eggs && !m ? "A 9-minute timer for hard-boiled eggs (6 minutes for soft), on the screen: press Start." :
        "A " + mins + "-minute timer, on the screen: press Start. I made it just now: on Chat Computer I can make any little page or app like this."),
        done: ["Made a timer"], act: function () { show({ html: timer(mins) }); } };
    }],
    [/\b(weather|rain|raining|forecast|temperature|umbrella|sunny|hot|cold) ?(today|tomorrow|outside|like|in|now)?\b/i, weather],
    // The news: looked up (and, with the browser's AI on, told in its words).
    [/\b(news|headlines)\b/i, function () { return { search: "news", news: true, later: newsAnswer }; }],
    // A website: right here when it lets this page show it (Google, Wikipedia: framed), else a link to it.
    [/^(open |go to |show me )?(https?:\/\/)?((?:[a-z0-9-]+\.)+(com|org|net|ai|io|co|sg|uk|de|fr|jp)(\.[a-z]{2})?)\b|^(google|youtube|gmail|facebook|instagram|netflix|wikipedia)[.!?]*$/i, function (t) {
      var m = /((?:[a-z0-9-]+\.)*[a-z0-9-]+\.(?:com|org|net|ai|io|co|sg|uk|de|fr|jp)(?:\.[a-z]{2})?)(\/[^\s]*)?/i.exec(t);
      var host = m ? m[1].replace(/^www\./i, "").toLowerCase() :
        { google: "google.com", youtube: "youtube.com", gmail: "mail.google.com", facebook: "facebook.com", instagram: "instagram.com",
          netflix: "netflix.com", wikipedia: "wikipedia.org" }[t.trim().replace(/[.!?]+$/, "").toLowerCase()];
      var url = "https://" + (/^(google\.com|wikipedia\.org)$/.test(host) ? "www." : "") + host + (m && m[2] ? m[2].replace(/[.!?,]+$/, "") : "/");
      var name = { "google.com": "Google", "wikipedia.org": "Wikipedia", "en.wikipedia.org": "Wikipedia" }[host] || host;
      if (framed(url)) {
        return { text: "Here's " + name + ". Click in it to type, and scroll it as you'd scroll any page.", done: ["Opened " + host],
                 act: function () { show(site(url, name)); } };
      }
      return { text: "Most sites can't show inside this page, " + host + " included, so it's a click away, on the screen. On a Chat Computer it opens right here, and I use it for you.",
        done: ["Showed a link to " + host], act: function () { show({ html: link(url, host, "On Chat Computer, " + host + " opens right here. Most sites can't show inside this page.") }); } };
    }],
    [/\b(reply|answer|send|write back|text)\b.*\b(mum|mom|jane|alex|her|him|them|message|email)\b/i, function (t) {
      var alex = /alex/i.test(t), jane = /jane/i.test(t);
      return { text: "I can't send messages: Chat Computer reads your messages, but replying isn't possible yet. " +
        (alex ? "Alex's" : jane ? "Jane's" : "Mum's") + " message is on the screen, so you can reply on your phone.",
        act: function () { openMessage(alex ? "m3" : jane ? "m2" : "m1"); } };
    }],
    [/\b(jane|lunch|email|emails|e-mail|mail|inbox|gmail|outlook)\b/i, function () {
      return { text: "Jane says 12:30 at the usual place works for her, for lunch on Friday. Her emails are on the screen.", act: function () {
        openMessage("m2");
      } };
    }],
    // Before the messages: "make the text bigger" isn't a text message.
    [/\b(bigger|larger|smaller|text size|font|zoom)\b/i, function (t) {
      var bigger = /bigger|larger|zoom in/i.test(t);
      return { text: bigger ? "Made the text bigger." : "Made the text smaller.", done: [bigger ? "Made the text bigger" : "Made the text smaller"],
        act: function () { setScale(scale * (bigger ? 1.15 : 1 / 1.15)); } };
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
      return { text: "Connect AirPods, at the right of the message box, and you can talk to me through them.", done: [] };
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
    if (MASH.test(text)) return { text: "I didn't catch that. " + TRY };
    if (LOOK_IT_UP.test(text) || SEARCH_FOR.test(text)) { var l = lookUp(text); if (l) return l; }
    if (OPEN_ENDED.test(text)) return { ai: true };
    for (var i = 0; i < INTENTS.length; i++) if (INTENTS[i][0].test(text)) return INTENTS[i][1](text);
    return FRESH.test(text) ? { ai: true, search: topicOf(text) } : { ai: true };
  }
  // A question only an AI can answer, with none here: a question of fact gets Wikipedia's article on it (the news, for
  // the news); the rest an honest note, and how to get an AI.
  var FACTUAL = /^\s*(who|what|when|where|which|whose|why|how (many|much|old|tall|big|far|long|does|do|did|is|are|was|were)|tell me about|explain|define)\b/i;
  function fallback(text) {
    if (/^[\W\d_]*[a-z]{0,6}[\W\d_]*$/i.test(text) && !/\b(hi|ok)\b/i.test(text)) {
      return { text: "I didn't catch that. " + TRY };
    }
    var chrome = nano.state === "downloadable" || nano.state === "downloading";
    var want = WANTS.filter(function (w) { return w[0].test(text); })[0];
    var note = { text: "I can't " + (want ? want[1] : "answer that") + " on this page without an AI: my answers here are written in advance. " +
      (chrome ? "Your " + AI.browser + " has a free AI built in that can answer on your device: press “Download " + AI.short + "” at the right of the message box. " : "") +
      REAL + " On a Chat Computer, Claude answers anything, and does it. " + TRY };
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

  // ------------------------------------------------------------------ the browser's built-in AI (Gemini Nano), free
  // Chrome on a computer (Windows, macOS, Linux) has a small AI built in, Gemini Nano, that runs on the device itself:
  // nothing typed leaves it. Where it's there (LanguageModel, Chrome's Prompt API), it answers what the written answers
  // can't, free, and its answer can also show something on the screen (a page it writes, a map, a timer...); it looks
  // things up on the web through this page (lookup, above). It has to be downloaded once, which Chrome does only after a
  // click: the "Download Gemini" button. Edge has the same with Phi-4-mini ("Download Phi-4-mini").
  var nano = { state: "none", session: null, creating: null, progress: 0 };
  var NANO_LANG = { expectedInputs: [{ type: "text", languages: ["en"] }], expectedOutputs: [{ type: "text", languages: ["en"] }] };
  function nanoCheck() {
    if (typeof LanguageModel === "undefined" || !LanguageModel.availability) return;
    LanguageModel.availability(NANO_LANG).then(function (a) {
      nano.state = a;
      if (a === "available") nanoSession().catch(function () {});
      drawTools();
    }, function () { nano.state = "unavailable"; });
  }
  // What it's told: who it is and where; that it can look things up (the page does it for it, see lookup); how to
  // answer; Chat Computer's facts; and what it can do on the screen.
  function nanoSystem() {
    return "You answer on Chat Computer's website (chatcomputer.ai), a demo of Chat Computer's screen. You are " + AI.model + ", " +
      AI.browser + "'s built-in AI, running on the visitor's device: you are not Claude. It's " + clock() + ".\n\n" +
      "You can look things up on the web: the page searches Wikipedia and the day's news for you and gives you what it finds. " +
      "Look up anything recent or that changes (news, events, sport, prices, who has a job now), anything after your training, " +
      "and facts you aren't sure of, and always when they ask you to look something up: action search, with a short query. " +
      "Then answer from what's found, and say where it's from. Never say you can't go on the internet or look things up. " +
      "Answer what you know well (explanations, writing, advice, maths, history) straight away.\n\n" +
      "Answer in the conversation, in English: in a sentence or two for a simple question, and in full when they ask for " +
      "more (a recipe, a plan, an email, a poem, code, an explanation), with a little Markdown where it helps (a list, a " +
      "code block). Links can't be clicked here: don't add any.\n\n" +
      "Facts about Chat Computer: " + FACTS.what + " " + FACTS.indie + " " + FACTS.free + " " + FACTS.soon + " " + FACTS.pc + " " +
      FACTS.mac + " Never make up facts about Chat Computer beyond these.\n\n" +
      "Actions: none; search (query); show_map (place); play_music (soft piano the browser plays); " +
      "youtube_search (query: a link to YouTube's search); open_website (url); open_message (id: m1 Mum on WhatsApp about dinner " +
      "at 7, m2 Jane about lunch on Friday, m3 Alex with the slides); open_file (name: Packing list.md, Beach.jpg, Budget " +
      "2026.xlsx, Taxes 2025.pdf, Recipes.txt, Resume.docx); make_timer (minutes); set_volume (percent); close. Their " +
      "messages and files are made up for the demo.";
  }
  // What the page tells it it found, for the question asked (the same words in its examples, below).
  function nanoFound(asked, q, list) {
    var lines = [], size = 0;
    list.forEach(function (x) {
      var line = (lines.length + 1) + ". " + x.source + (x.title ? ", " + x.title : "") + ": " + clip(x.text, x.air ? 300 : x.links ? 260 : 700);
      if (size + line.length <= 3600) { lines.push(line); size += line.length; }
    });
    return "The question: " + asked + "\nWhat the page found on the web for “" + q + "”, just now:\n" +
      (lines.length ? lines.join("\n") : "Nothing.") + "\n\nAnswer the question from this, briefly unless they asked for more, " +
      "and say where it's from (like “according to Wikipedia”). If it doesn't answer it, say what it does say, or that " +
      "nothing was found.";
  }
  // Examples, for a small model: one it knows, one it looks up and answers from what's found, one it writes.
  var NANO_EXAMPLES = [
    ["What's the capital of Japan?", { reply: "Tokyo.", action: "none" }],
    ["How tall is Mount Everest now? Didn't it change?", { reply: "Let me look that up.", action: "search", query: "Mount Everest height" }],
    [nanoFound("How tall is Mount Everest now? Didn't it change?", "Mount Everest height", [{ source: "Wikipedia", title: "Mount Everest",
      text: "Mount Everest is Earth's highest mountain above sea level. It lies in the Mahalangur Himal sub-range of the Himalayas. Its " +
      "elevation (snow height) of 8,848.86 m was most recently established in 2020 by the Chinese and Nepali authorities." }]),
     { reply: "It's 8,849 metres, measured again in 2020 by China and Nepal, according to Wikipedia.", action: "none" }],
    ["write a haiku about coffee", { reply: "Dark cup in my hands,\nsteam curling into morning,\nthe day starts with you.", action: "none" }]
  ];
  function nanoSession(monitor) {
    if (nano.session) return Promise.resolve(nano.session);
    if (!nano.creating) {
      var opts = { initialPrompts: [{ role: "system", content: nanoSystem() }] };
      NANO_EXAMPLES.forEach(function (e) {
        opts.initialPrompts.push({ role: "user", content: e[0] }, { role: "assistant", content: JSON.stringify(e[1]) });
      });
      for (var k in NANO_LANG) opts[k] = NANO_LANG[k];
      if (monitor) opts.monitor = monitor;
      nano.creating = LanguageModel.create(opts).then(function (s) {
        nano.session = s;
        nano.state = "available";
        drawTools();
        return s;
      }, function (e) { nano.creating = null; throw e; });
    }
    return nano.creating;
  }
  function nanoButton() {
    if (nano.state === "available" || nano.state === "downloading" && nano.creating) return;
    call("accountForm", { id: "nano", title: "Download " + AI.short, ok: "Download it", text: [
      AI.browser + " has a small AI built in, " + AI.model + ", that can answer here for free, and look things up on Wikipedia for " +
      "you. It runs on this computer: what you type doesn't leave it, only the words it looks up.",
      AI.browser + " downloads it once (a few gigabytes, from " + AI.maker + ") and keeps it for every site that uses it. The written answers " +
      "work meanwhile.",
      "For answers from Claude itself, use Connect Claude instead."] });
  }
  function nanoDownload() {
    call("accountForm", null);
    nano.state = "downloading";
    drawTools();
    nanoSession(function (m) {
      m.addEventListener("downloadprogress", function (e) { nano.progress = Math.round((e.loaded || 0) * 100); drawTools(); });
    }).then(function () { did("Turned on " + AI.browser + "'s built-in AI"); }, function (e) {
      nano.state = "downloadable";
      drawTools();
      call("accountForm", { id: "nano-no", title: AI.browser + "'s AI didn't start", ok: "OK", text: [
        AI.browser + " said: " + clip(e && e.message || "it couldn't", 160) + ".",
        "It needs " + AI.browser + " on a computer with enough room and memory (see " + AI.browser + "'s built-in AI requirements). " +
        "The written answers still work."] });
    });
  }
  var NANO_SCHEMA = { type: "object", properties: {
    reply: { type: "string" },
    action: { type: "string", enum: ["none", "search", "show_map", "play_music", "youtube_search", "open_website",
                                     "open_message", "open_file", "make_timer", "set_volume", "close"] },
    title: { type: "string" }, markdown: { type: "string" }, place: { type: "string" }, query: { type: "string" },
    url: { type: "string" }, id: { type: "string" }, name: { type: "string" }, minutes: { type: "integer" },
    percent: { type: "integer" } }, required: ["reply", "action"] };
  // Its answer from what was found, in the conversation (no looking up again, nothing else done).
  var NANO_ANSWER = { type: "object", properties: { reply: { type: "string" }, action: { type: "string", enum: ["none"] } },
    required: ["reply", "action"] };
  // What its answer says to show, done here (the same as the written answers do), and what to add to its words.
  function nanoAct(r, you, uts) {
    var a = r.action;
    if (a === "show_page" && r.markdown) {
      show({ html: doc(clip(r.title || "Page", 80), String(r.markdown), "Written by " + AI.browser + "'s built-in AI, on this device.") });
      did("Showed “" + clip(r.title || "a page", 60) + "”", you, uts);
    } else if (a === "show_map" && r.place) {
      return Promise.resolve(mapOf("map of " + r.place)).then(function (m) { return m.later ? m.later() : m; })
        .then(function (m) { if (m.act) m.act(); (m.done || []).forEach(function (d) { did(d, you, uts); }); return ""; });
    } else if (a === "play_music") { piano(); did("Played some calm piano", you, uts); }
    else if (a === "youtube_search" && r.query) { var y = youtubeSearch(r.query); y.act(); y.done.forEach(function (d) { did(d, you, uts); }); }
    else if (a === "open_website" && /^https?:\/\/[^\s]+$/i.test(r.url || "")) {
      var host = r.url.replace(/^https?:\/\/(www\.)?/i, "").split(/[/?#]/)[0];
      show(site(r.url, host, "On Chat Computer, " + host + " opens right here. Most sites can't show inside this page."));
      did((framed(r.url) ? "Opened " : "Showed a link to ") + host, you, uts);
    } else if (a === "open_message" && THREADS[r.id]) openMessage(r.id);
    else if (a === "open_file") {
      var f = FILES.filter(function (x) { return x.name.toLowerCase() === String(r.name || "").toLowerCase(); })[0];
      if (f) { show({ html: PAGES[f.page]() }); did("Opened " + f.name, you, uts); }
    } else if (a === "make_timer") {
      var mins = Math.max(1, Math.min(180, parseInt(r.minutes, 10) || 10));
      show({ html: timer(mins) });
      did("Made a timer", you, uts);
    } else if (a === "set_volume" && r.percent != null) setVolume(Number(r.percent));
    else if (a === "close") close();
    return Promise.resolve("");
  }
  // The session's turns, one at a time and in order: its own, and the written answers it's told about (nanoNote).
  function nanoDo(f) {
    var next = (nano.queue || Promise.resolve()).then(function () { return nanoSession(); }).then(f);
    nano.queue = next.catch(function () {});
    return next;
  }
  function nanoPrompt(input, schema) {
    return nanoDo(function (s) {
      return s.prompt(input, { responseConstraint: schema }).catch(function (e) {
        if (e && /quota|context|too (long|large)/i.test(e.name + " " + e.message)) {       // the conversation's grown too long:
          try { s.destroy(); } catch (x) {}                                                // start it again
          nano.session = null;
          nano.creating = null;
          return nanoSession().then(function (s2) { return s2.prompt(input, { responseConstraint: schema }); });
        }
        throw e;
      });
    }).then(function (raw) {
      try { return JSON.parse(raw); } catch (e) { return { reply: String(raw || ""), action: "none" }; }
    });
  }
  // A written answer goes into its conversation as if it had said it, so what's asked next makes sense to it ("the
  // haze?" after the weather).
  function nanoNote(asked, said) {
    if (nano.state !== "available" || !said) return;
    nanoDo(function (s) {
      if (typeof s.append !== "function") return null;
      return s.append([{ role: "user", content: asked }, { role: "assistant", content: JSON.stringify({ reply: said, action: "none" }) }]);
    }).catch(function () {});
  }
  // One turn: it answers, or says what to look up; then the page looks it up (in Actions) and it answers from what
  // was found. wants: what the written answers know already: that it's a search (query: for what), or the news.
  function nanoTurn(text, you, uts, id, wants, since) {
    wants = wants || {};
    call("aiStart", id);
    var looked = null;            // what was found, once looked up
    (wants.news ? Promise.resolve({ action: "search", query: wants.query }) : nanoPrompt(text, NANO_SCHEMA)).then(function (r) {
      // Its own words for what to look up know the conversation ("Singapore haze", for "the haze?"); else the page's.
      var q = r.action === "search" || wants.search ? r.query || wants.query || topicOf(text) : "";
      if (!q) return r;
      q = clip(String(q), 100);
      return lookup(q, text).then(function (got) {
        looked = got;
        got.told.forEach(function (d) { did(d, you, uts); });
        // Chrome's model sometimes fails on what was found (an "unknown error"): again with less of it, in Latin letters.
        return nanoPrompt(nanoFound(text, q, got.list), NANO_ANSWER).catch(function () {
          var less = got.list.slice(0, 3).map(function (x) {
            return { source: x.source, title: x.title, air: x.air, links: x.links, text: clip(x.text.replace(/[^\u0000-ɏ‘-”…°µ³]+/g, " "), 300) };
          });
          return nanoPrompt(nanoFound(text, q, less), NANO_ANSWER);
        }).catch(function () { return fromFound(got, q); });
      });
    }).then(function (r) {
      if (r.written) { deliver(r.written, id, you, uts, since); return; }
      // Its words go in the conversation as they are, Markdown and all (a JSON copy of its answer in them is dropped).
      var words = String(r.reply || "").replace(/```json[\s\S]*?(```|$)/g, "").trim();
      // Where it's from, only when it was looked up: from memory, it doesn't say "according to Wikipedia".
      if (!looked) words = unsourced(words);
      words = words || "Done.";
      return Promise.resolve(nanoAct(r, you, uts)).then(function () {
        if (since != null) wordsOnly(since);
        stream(id, words, function () { say(words); });
      });
    }).catch(function () {
      var f = fallback(text);
      Promise.resolve(f.later ? f.later() : f).then(function (a) { deliver(a, id, you, uts, since); });
    });
  }
  function unsourced(s) {
    var t = s.replace(/^\s*(according to|from|per) (wikipedia|the news|wikipedia's current events|open-?meteo)[^,]*,\s*/i, "")
      .replace(/,?\s*(according to|from|per|as reported by) (wikipedia|the news|wikipedia's current events|open-?meteo)[^.!?]*/gi, "");
    return t.charAt(0).toUpperCase() + t.slice(1);
  }
  // Its answer failed: the page says what was found itself (the air, the news, or Wikipedia's article).
  function fromFound(got, q) {
    var air_ = got.list.filter(function (x) { return x.air; })[0], news_ = got.list.filter(function (x) { return x.links; }),
        wiki = got.list.filter(function (x) { return x.source === "Wikipedia"; });
    var a = air_ ? { text: air_.text + " (Open-Meteo's estimate, from the Copernicus forecast.)" }
      : news_.length ? { text: newsWords(news_) }
      : wiki.length ? { text: wikiWords(wiki[0]) }
      : null;
    if (!a) throw new Error("nothing found");
    return { written: a };
  }
  function setScale(s) {
    scale = Math.max(0.6, Math.min(2.2, s));
    store("chatcomputer-demo-scale", scale);
    layout();
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
  function ask(text) {
    var you = uid("u"), uts = now(), id = "a" + (++turns), since = acted;
    call("user", you, text, true);
    if (claude.key) { claudeTurn(text, you, uts, id, since); return; }
    var r = answer(text), ai = nano.state === "available";
    if (!LOOK_IT_UP.test(text)) lastAsked = text;              // what "look it up" means next
    // The browser's AI answers what only an AI can, and what's looked up (in its words, from what's found).
    if (ai && (r.ai || r.search)) {
      setTimeout(function () { nanoTurn(text, you, uts, id, r.search ? { search: true, query: r.search, news: r.news } : null, since); }, 120);
      return;
    }
    if (r.ai) r = fallback(text);
    setTimeout(function () {
      call("aiStart", id);
      var waited = Date.now();
      Promise.resolve(r.later ? r.later() : r).then(function (a) {
        setTimeout(function () {
          deliver(a, id, you, uts, since);
          if (ai) nanoNote(text, a.text);
        }, Math.max(0, 600 - (Date.now() - waited)));
      });
    }, 120);
  }
  // A written answer: what it shows, what it did (Actions), and its words (where they can be read: wordsOnly).
  function deliver(a, id, you, uts, since) {
    if (a.act) a.act();
    (a.done || []).forEach(function (d) { did(d, you, uts); });
    if (a.text && since != null) wordsOnly(since);
    stream(id, a.text || "", function () { say(a.text || ""); });
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
  // The model a Chat Computer uses (chatcomputer/config.py), but at the API's own effort (medium) rather than the
  // computer's max, for quicker answers. It always thinks first, and max_tokens counts the thinking too.
  var MODEL = "claude-opus-5-5", MODEL_NAME = "Opus 5.5", MAX_TOKENS = 16000;
  var KEY = "chatcomputer-demo-claude-key";
  // talk: the conversation, as sent; system: its system prompt, made when it starts and kept as it is, as the
  // thinking Claude sends back is only valid while the system prompt, the tools and the messages before it don't change.
  var claude = { key: store(KEY) || "", talk: [], system: "", search: true };
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
  function clock() {
    var zone = "";
    try { zone = Intl.DateTimeFormat().resolvedOptions().timeZone || ""; } catch (e) {}
    return new Date().toLocaleString("en-GB", { weekday: "long", day: "numeric", month: "long", year: "numeric", hour: "numeric",
                                               minute: "2-digit" }) + (zone ? ", " + zone : "");
  }
  function systemPrompt() {
    return "You are Chat Computer: Claude, running a whole computer. People talk, type or touch, and you do things for them. " +
      "Right now you're in Chat Computer's demo website (chatcomputer.ai), in the person's web browser, with their own Claude " +
      "API key. The screen is Chat Computer's: Messages at the left, Actions at the right (each thing you do is listed " +
      "there), and in the middle the conversation, or what you open in its place, with the message box under it. While " +
      "something is open the conversation is out of sight: a reply that's only words brings it back, putting what's open " +
      "away (still playing).\n\n" +
      "Answer in the conversation, like a chat: briefly for simple things, in full when they ask for more (a recipe, a " +
      "plan, code), with a little Markdown where it helps (a list, a table, a code block; links can't be clicked here). " +
      "When they talk to you (through their AirPods) it's read aloud too, so keep it to a few sentences then. Do things " +
      "rather than describe them, and don't announce a tool before using it.\n\n" +
      "Here you can: play YouTube videos (play_youtube; use web_search to find a video's address when you don't know its " +
      "ID for certain), show maps (show_map), show a page you write when they ask for one on the screen (show_page), open a website (open_website: " +
      "Google, its searches and Google Maps' places and directions, and Wikipedia open right here, for them to scroll and type in; " +
      "most other sites can't show inside the demo, so it's a link; on a Chat Computer any site opens right here and you use it), open their " +
      "messages and files (open_message, open_file), make a timer (make_timer), set the volume of what's playing " +
      "(set_volume), close what's open (close), and search the web. On a real Chat Computer you also use any website, " +
      "their real files, apps and settings, and their AirPods; when they ask for something only a Chat Computer can do, " +
      "say so in a sentence.\n\n" +
      "Their messages and files here are made up for the demo:\n" + inbox() + "\n\nFiles:\n" +
      FILES.map(function (f) { return "- " + f.name + ", in " + f.where; }).join("\n") +
      "\n\nEach of their messages starts with the date and time it was sent, in brackets.";
  }
  function obj(props, required) { return { type: "object", properties: props, required: required || [] }; }
  var TOOLS = [
    { name: "play_youtube", description: "Play a YouTube video between the sidebars. The demo can't search YouTube itself: find the " +
      "video's address with web_search unless you know its ID for certain.", input_schema: obj({ video_id: { type: "string", description: "The 11-character ID, as in youtube.com/watch?v=ID" },
      title: { type: "string", description: "The video's title, for Actions" }, start: { type: "integer", description: "Seconds in to start at" } }, ["video_id", "title"]) },
    { name: "show_map", description: "Show a map (OpenStreetMap) between the sidebars, of the area in the box given in degrees.",
      input_schema: obj({ place: { type: "string" }, west: { type: "number" }, south: { type: "number" }, east: { type: "number" },
      north: { type: "number" } }, ["place", "west", "south", "east", "north"]) },
    { name: "show_page", description: "Show a page you write between the sidebars, in Markdown (headings, lists, tables), when " +
      "they ask for something on the screen; answers go in the conversation.", input_schema: obj({ title: { type: "string" }, markdown: { type: "string" } }, ["title", "markdown"]) },
    { name: "open_website", description: "Open a website. Google (google.com, its searches, Google Maps' places and directions) and " +
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
      show({ html: page(host, '<article><h1 class="title">' + esc(a.title || host) + '</h1><p class="demo-note">On Chat Computer, ' + esc(host) +
        " opens right here, and Claude uses it for you. Most sites can't show inside this demo, so here's the link.</p><p><a href=\"" + esc(url) +
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
    if (n === "close") { var was = !!view; close(); return was ? "Closed." : "Nothing was open."; }
    return "There's no tool called " + n + " here.";
  }
  // One turn: Claude's answer streams in, its tools run here, and it goes on until it's done (at most 8 rounds).
  function claudeTurn(text, you, uts, id, since) {
    var said = "", gap = false, rounds = 0, asked = { role: "user", content: "[" + clock() + "] " + text };
    if (!claude.talk.length) claude.system = systemPrompt();
    claude.talk.push(asked);
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
      message(onText, you, uts, claude.talk.indexOf(asked)).then(function (msg) {
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
  // The conversation as sent: the thinking of turns already over is left out (allowed outside a turn's tool use, and
  // still right after older turns are trimmed away); this turn's, from its question on, goes back as it came.
  function sent(from) {
    return claude.talk.map(function (m, i) {
      if (i >= from || m.role !== "assistant" || typeof m.content === "string") return m;
      var c = m.content.filter(function (b) { return b.type !== "thinking" && b.type !== "redacted_thinking"; });
      return { role: "assistant", content: c.length ? c : [{ type: "text", text: "\u2026" }] };
    });
  }
  // One request, streamed: text as it comes, and the whole message (its blocks, cleaned) at the end.
  function message(onText, you, uts, from) {
    var body = { model: MODEL, max_tokens: MAX_TOKENS, stream: true,
                 system: [{ type: "text", text: claude.system || systemPrompt(), cache_control: { type: "ephemeral" } }],
                 tools: tools(), messages: sent(from) };
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
  // Read aloud, under a message: the browser says it now (in place of anything it was saying).
  function readAloud(text) {
    if (!window.speechSynthesis || !text) return;
    var plain = String(text).replace(/[*_`#>|~]/g, "").replace(/\s+/g, " ").trim();
    try { window.speechSynthesis.cancel(); } catch (e) {}
    var u = new SpeechSynthesisUtterance(plain);
    u.lang = navigator.language || "en-US";
    window.speechSynthesis.speak(u);
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
  var SPARK_ICON = '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 3l1.8 5.2L19 10l-5.2 1.8L12 17l-1.8-5.2L5 10l5.2-1.8z"/><path d="M19 15l.8 2.2L22 18l-2.2.8L19 21l-.8-2.2L16 18l2.2-.8z"/></svg>';
  var tools_ = null;
  function drawTools() {
    if (!tools_) {
      tools_ = document.createElement("div");
      tools_.id = "demo-tools";
      tools_.innerHTML = '<button type="button" id="demo-nano" tabindex="-1" hidden>' + SPARK_ICON + '<span class="label"></span></button>' +
        '<button type="button" id="demo-claude" tabindex="-1">' + KEY_ICON + '<span class="label"></span></button>' +
        '<button type="button" id="demo-airpods" tabindex="-1">' + PODS_ICON + '<span class="dot"></span><span class="label"></span></button>';
      $("box").appendChild(tools_);
      tools_.addEventListener("mousedown", function (e) { e.preventDefault(); });       // typing stays in the box
      $("demo-claude").addEventListener("click", claudeButton);
      $("demo-nano").addEventListener("click", nanoButton);
      $("demo-airpods").addEventListener("click", talkButton);
      // The box or the room beside it changed (the sidebars switched off, Escape filling the screen): placed again.
      var ro = new ResizeObserver(function () { requestAnimationFrame(placeTools); });
      ro.observe($("box"));
      ro.observe($("composer"));
    }
    var c = $("demo-claude"), p = $("demo-airpods"), g = $("demo-nano");
    // Chrome's AI: offered while it can be downloaded, its progress while it is; once on, it just answers.
    g.hidden = !!claude.key || !(nano.state === "downloadable" || nano.state === "downloading");
    label(g, nano.state === "downloading" ? ["", "Downloading", nano.progress ? " " + nano.progress + "%" : ""] : ["Download ", AI.short, ""]);
    g.title = nano.state === "downloading" ? AI.browser + " is downloading its built-in AI" : "Download " + AI.model + ", " + AI.browser +
      "'s built-in AI: free answers, on this device";
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
    tools_.classList.remove("stack", "short", "compact", "inside", "rise");
    if (tools_.parentNode !== box) box.appendChild(tools_);
    var b = box.getBoundingClientRect(), limit = $("composer").getBoundingClientRect().right;
    var room = limit - b.right - 1.25 * rem;
    // A stack taller than the box (three buttons) that would reach the window's bottom edge rises from the box's bottom.
    function rise() { if (tools_.getBoundingClientRect().bottom > innerHeight - 0.25 * rem) tools_.classList.add("rise"); }
    if (tools_.offsetWidth <= room) return;
    tools_.classList.add("stack");
    if (tools_.offsetWidth <= room) { rise(); return; }
    tools_.classList.add("short");
    if (tools_.offsetWidth <= room) { rise(); return; }
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
        nanoCheck();
        break;
      case "send": ask(String(msg.text || "")); break;
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
      // Under a message: read it aloud; good or bad (kept in this browser only: nothing leaves it).
      case "speak": readAloud(String(msg.text || "")); break;
      case "feedback": {
        var fb = store("chatcomputer-demo-feedback") || [];
        fb.push({ ts: now(), good: !!msg.good, on: !!msg.on, prompt: clip(msg.prompt, 300), answer: clip(msg.text, 600) });
        store("chatcomputer-demo-feedback", fb.slice(-100));
        break;
      }
      case "search": call("searchResults", msg.n, msg.q || "", search(msg.q || ""),
                          search(msg.q || "").length ? "" : "Nothing in the demo's files matches \u201c" + (msg.q || "") + "\u201d.");
        break;
      case "open_file": if (PAGES[msg.path]) show({ html: PAGES[msg.path]() }); break;
      case "text_scale": setScale(scale * (+msg.factor || 1)); break;
      case "panels": store("chatcomputer-demo-panels", { left: !!msg.left, right: !!msg.right }); break;
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
        if (msg.form === "nano") { nanoDownload(); break; }
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
