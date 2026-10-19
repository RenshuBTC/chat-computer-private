/* ClaudeOS in a browser: the demo on ClaudeOS's GitHub page.

   The screen here is ClaudeOS's own, the same page and code as on a ClaudeOS computer (chat.html, chat.js,
   chat.css: copied here by scripts/build_demo.py). This file stands in for the computer behind it. On ClaudeOS,
   what you type (or say, or type in the Claude app on your phone) goes to Claude, which opens websites, plays
   music, reads your messages and looks after your files. Here, Claude's answers to a few things are written in
   advance, the messages and files are made up, and everything stays in your browser: nothing typed here is sent
   anywhere. (The one thing fetched from elsewhere is the map, from OpenStreetMap, when you ask for one.) */
(function () {
  "use strict";
  var START = Date.now() / 1000;
  function now() { return Date.now() / 1000; }
  function call(name) {
    var args = Array.prototype.slice.call(arguments, 1);
    if (window.CO && typeof window.CO[name] === "function") window.CO[name].apply(null, args);
  }
  function esc(s) {
    return String(s == null ? "" : s).replace(/[&<>"']/g, function (c) {
      return { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c];
    });
  }
  function store(key, value) {
    try {
      if (value === undefined) return JSON.parse(window.localStorage.getItem(key) || "null");
      window.localStorage.setItem(key, JSON.stringify(value));
    } catch (e) { return null; }
    return null;
  }

  // What chat.js says to ClaudeOS goes here (on a ClaudeOS computer, to its Python side).
  window.webkit = { messageHandlers: { claudeos: { postMessage: function (s) {
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
    m1: { title: "Mum", note: "On ClaudeOS this is WhatsApp's own page, between the sidebars.", lines: [
      ["Mum", START - 90000, "How was the trip?"], ["You", START - 89000, "Amazing! So many photos."],
      ["Mum", START - 240, "Dinner at 7 tonight? Bring the photos from the trip!"]] },
    m2: { title: "Jane Lim", mail: "Jane Lim <jane@example.com>", mails: [
      ["Lunch on Friday?", START - 180000, "Hi! Are you free for lunch on Friday? There's a new noodle place near the office.\n\nJane"],
      ["Re: Lunch on Friday?", START - 3100, "12:30 at the usual place works for me. See you then!\n\nJane", true]] },
    m3: { title: "Alex", lines: [["Alex", START - 7500, "Do you have the slides?"], ["You", START - 7450, "Not yet"],
      ["Alex", START - 7400, "Sent you the slides for Monday."]] },
    m4: { title: "sam.k", note: "On ClaudeOS this is Instagram's own page.", lines: [["sam.k", START - 11000, "Liked your photo"]] },
    m5: { title: "City Library", mail: "City Library <noreply@library.example>", mails: [
      ["Your books are due soon", START - 90000, "Your books are due back on Thursday:\n\n- The Overstory\n- A Field Guide to Getting Lost\n\nRenew them online or at any branch.", true]] },
    m6: { title: "Priya", note: "On ClaudeOS this is X's own page.", lines: [["Priya", START - 96000, "Are you coming to the meetup on Saturday?"]] },
    m7: { title: "Daniel Tan", note: "On ClaudeOS this is LinkedIn's own page.", lines: [["Daniel Tan", START - 260000, "Thanks for connecting!"]] }
  };
  var HISTORY = [
    { role: "user", id: "h1", ts: START - 5410, text: "How do I get to the airport on Sunday with two big suitcases?" },
    { role: "assistant", id: "ha1", ts: START - 5380, text: "The train means two changes, and one of them has stairs and no lift, so a taxi is easier: about 25 minutes. I put a packing list in your Documents too." }
  ];
  var EARLIER = [
    { role: "user", id: "y1", ts: START - 86400 - 3010, text: "Play some calm piano music" },
    { role: "assistant", id: "ya1", ts: START - 86400 - 3000, text: "Playing a calm piano mix on YouTube." },
    { role: "user", id: "y2", ts: START - 86400 - 2910, text: "A bit quieter please" },
    { role: "assistant", id: "ya2", ts: START - 86400 - 2900, text: "Volume is 40%." }
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
  // On ClaudeOS the text size comes from the screen's real size and how far away you sit (ui/layout.py); here,
  // from the window's size. Pinching the conversation (a touchscreen) changes it, as it does there.
  var size = 20, column = 600, scale = store("claudeos-demo-scale") || 1;
  function layout() {
    var w = window.innerWidth, h = window.innerHeight;
    size = Math.max(15, Math.min(26, Math.min(w * 0.0125, h * 0.022))) * scale;
    column = Math.round(Math.min(30 * size, w - 2 * size));
    call("layout", Math.round(size * 10) / 10, column);
    place();
  }
  window.addEventListener("resize", layout);

  // ------------------------------------------------------------------ what's open between the sidebars
  // On ClaudeOS a second browser view, laid over the space the conversation page leaves for it ("hole"); here a
  // frame. F1 makes it fill the window, F1 again puts it back; Escape closes it.
  var view = null, open = false, expanded = false, hole = null, drafting = false;
  function frame() {
    if (view) return view;
    view = document.createElement("iframe");
    view.id = "demo-view";
    view.title = "What's open";
    view.setAttribute("allow", "fullscreen; autoplay");
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
  // A touchscreen, as an iPad's: swipe up from the bottom edge to close what's open (Home), down from the top for
  // Messages. (On ClaudeOS the window does these, and more: three fingers copy and paste, the sides go back.)
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

  // Pages shown there: ClaudeOS's look (theme.css, and the pages' own doc.css from ui/render.py).
  function page(title, body, css, script) {
    return '<!doctype html><html lang="en"><head><meta charset="utf-8"><title>' + esc(title) + "</title>" +
      '<link rel="stylesheet" href="demo/theme.css"><link rel="stylesheet" href="demo/doc.css"><style>:root{--co-size:' +
      size.toFixed(1) + "px;--co-column:" + column + "px}" + (css || "") + "</style></head><body>" + body +
      (script ? "<script>" + script + "<\/script>" : "") + "</body></html>";
  }
  function doc(title, markdown, note) {
    return page(title, '<article><h1 class="title">' + esc(title) + "</h1>" + (note ? '<p class="demo-note">' + esc(note) + "</p>" : "") +
      '<div class="md">' + window.renderMarkdown(markdown) + "</div></article>",
      ".demo-note{margin:0 0 1.25rem;padding:0.5rem 0.75rem;border:2px solid var(--bubble);border-radius:0.6rem}");
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
      ".demo-note{margin:0 0 1.25rem;padding:0.5rem 0.75rem;border:2px solid var(--bubble);border-radius:0.6rem}" +
      ".said.mail{padding:0 0 1rem;margin:0 0 1.25rem;border-bottom:2px solid var(--bubble)}.said.mail:last-child{border-bottom:0}",
      "var o=document.getElementById('open');if(o)o.scrollIntoView({block:'start'});");
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
        "A made-up spreadsheet. On ClaudeOS, files on Windows' drive open too.");
    },
    taxes: function () {
      return doc("Taxes 2025.pdf", "## Notice of assessment, 2025\n\nThe pages of a PDF show here as they are.",
        "A made-up document. On ClaudeOS, a PDF shows as its real pages.");
    },
    recipes: function () {
      return doc("Recipes.txt", "## Laksa for four\n\n1. Fry the laksa paste in a little oil for two minutes.\n" +
        "2. Add the coconut milk and stock; simmer for ten minutes.\n3. Add the prawns and tofu puffs; cook for three minutes.\n" +
        "4. Pour over the noodles, with bean sprouts, a boiled egg and laksa leaves.");
    },
    resume: function () {
      return doc("Resume.docx", "## Experience\n\n- **Product designer**, 2022 to now\n- **Designer**, 2019 to 2022\n\n" +
        "## Skills\n\nResearch, prototyping, writing.", "A made-up document. On ClaudeOS, Word files show as written.");
    }
  };

  // ------------------------------------------------------------------ music, made by the browser
  // On ClaudeOS, "play some piano music" opens YouTube and plays it. Here, the page that opens plays soft chords
  // made by the browser itself (or, where a browser waits for a tap first, its Play button does).
  var MUSIC = "var ctx=null,timer=0,btn=document.getElementById('play');" +
    "function play(){var C=window.AudioContext||window.webkitAudioContext;if(!C)return;ctx=new C();var out=ctx.createGain();" +
    "out.gain.value=0.18;out.connect(ctx.destination);var chords=[[60,64,67,71],[57,60,64,67],[53,57,60,64],[55,59,62,65]]," +
    "beat=0.42,t0=ctx.currentTime+0.1,bar=0;function note(m,at,len){var o=ctx.createOscillator(),g=ctx.createGain();" +
    "o.type='triangle';o.frequency.value=440*Math.pow(2,(m-69)/12);g.gain.setValueAtTime(0,at);" +
    "g.gain.linearRampToValueAtTime(0.5,at+0.02);g.gain.exponentialRampToValueAtTime(0.001,at+len);o.connect(g);g.connect(out);" +
    "o.start(at);o.stop(at+len+0.05);}function sched(){var c=chords[bar%4],at=t0+bar*8*beat;" +
    "[0,1,2,3,2,1,2,3].forEach(function(k,i){note(c[k]+(i?0:-12),at+i*beat,1.6);});bar++;}sched();sched();" +
    "timer=setInterval(sched,8*beat*1000);btn.textContent='Pause';}" +
    "function stop(){clearInterval(timer);if(ctx)ctx.close();ctx=null;btn.textContent='Play';}" +
    "btn.onclick=function(){if(ctx)stop();else play();};play();" +
    "setTimeout(function(){if(ctx&&ctx.state!=='running')stop();},400);";

  // ------------------------------------------------------------------ Claude's answers, written in advance
  var CITIES = { singapore: [103.6, 1.2, 104.1, 1.47], london: [-0.35, 51.38, 0.15, 51.62], "new york": [-74.1, 40.6, -73.8, 40.85],
                 tokyo: [139.55, 35.55, 139.95, 35.8], paris: [2.22, 48.8, 2.47, 48.92], sydney: [151.0, -33.98, 151.35, -33.78] };
  var TRY = "Try \u201cplay some piano music\u201d, \u201cshow me a map of Tokyo\u201d, \u201cmake me a timer\u201d, " +
            "\u201cwhat did Jane say?\u201d or \u201copen my packing list\u201d. Type / to search the files.";
  var INTENTS = [
    [/\b(hi|hello|hey|good (morning|afternoon|evening))\b|^help\b|what can you do|how does (this|it) work|what is (this|claudeos)/i, function () {
      return { text: "Hi! I'm ClaudeOS: Claude, running the whole computer. You talk, type or touch, and I do it: open websites, " +
        "play music, read your messages, look after your files and settings.\n\nIn this demo my answers are written in advance. " + TRY };
    }],
    [/\b(music|song|songs|piano|play|playing|spotify|youtube|lofi|jazz)\b/i, function () {
      return { text: "Playing some calm piano.", done: ["Played some calm piano"], act: function () {
        show({ html: page("Now playing", '<article><h1 class="title">Calm piano</h1><p>On ClaudeOS this is YouTube, playing ' +
          "what you asked for. Here, your browser plays a few soft chords.</p><p><button id=\"play\" class=\"b\">Play</button></p>" +
          "<p>Escape, or a swipe up from the bottom edge, closes it; F1 makes it fill the screen.</p></article>",
          ".b{font:inherit;padding:0.4em 1.2em;border:2px solid var(--ink);border-radius:0.6em;background:var(--ground);color:var(--ink);cursor:pointer}",
          MUSIC) });
      } };
    }],
    [/\b(map|maps|where is|directions|how far)\b/i, function (t) {
      var name = Object.keys(CITIES).filter(function (c) { return t.toLowerCase().indexOf(c) >= 0; })[0] || "singapore", b = CITIES[name];
      var label = name.replace(/\b\w/g, function (c) { return c.toUpperCase(); });
      return { text: "Here's " + label + ".", done: ["Opened openstreetmap.org"], act: function () {
        show({ src: "https://www.openstreetmap.org/export/embed.html?bbox=" + b.join("%2C") + "&layer=mapnik" });
      } };
    }],
    [/\b(timer|countdown|stopwatch)\b/i, function (t) {
      var m = /(\d+)\s*(min|minute)/i.exec(t), mins = m ? Math.min(180, +m[1]) : 10;
      return { text: "A " + mins + "-minute timer, on the screen. I made it just now: on ClaudeOS I can make any little page " +
        "or app like this.", done: ["Made a timer"], act: function () {
        show({ html: page("Timer", '<article style="text-align:center"><h1 class="title">Timer</h1><p id="left" style="font-size:4rem;' +
          'font-variant-numeric:tabular-nums;margin:1rem 0">' + mins + ':00</p><p><button id="go" class="b">Start</button> ' +
          '<button id="reset" class="b">Reset</button></p></article>',
          ".b{font:inherit;padding:0.4em 1.2em;border:2px solid var(--ink);border-radius:0.6em;background:var(--ground);color:var(--ink);cursor:pointer}",
          "var total=" + mins * 60 + ",left=total,run=null,el=document.getElementById('left'),go=document.getElementById('go');" +
          "function draw(){el.textContent=Math.floor(left/60)+':'+('0'+left%60).slice(-2)+(left?'':' Done!');}" +
          "go.onclick=function(){if(run){clearInterval(run);run=null;go.textContent='Start';}else{run=setInterval(function(){if(left>0)left--;draw();" +
          "if(!left){clearInterval(run);run=null;go.textContent='Start';}},1000);go.textContent='Pause';}};" +
          "document.getElementById('reset').onclick=function(){left=total;draw();};") });
      } };
    }],
    [/\b(weather|rain|raining|forecast|temperature|umbrella|sunny)\b/i, function () {
      return { text: "I can't look things up in this demo, so here's a made-up forecast to show where it goes: a short answer here, " +
        "the details on the screen.", act: function () {
        show({ html: doc("Weather", "| | Morning | Afternoon | Evening |\n|---|---|---|---|\n| Today | 27\u00b0, cloudy | 31\u00b0, showers | 28\u00b0, clear |\n" +
          "| Tomorrow | 26\u00b0, clear | 32\u00b0, sunny | 28\u00b0, clear |", "Made up for the demo. On ClaudeOS this is today's real forecast.") });
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
      return { text: up ? "Volume is 60%." : "Volume is 40%.", done: [up ? "Turned the volume up" : "Turned the volume down"] };
    }],
    [/\b(wi-?fi|internet|network)\b/i, function () {
      return { text: "You're on Home Wi-Fi, and the internet's working.", done: [] };
    }],
    [/\b(airpods|bluetooth|headphones|earbuds)\b/i, function () {
      return { text: "Your AirPods are connected.", done: ["Connected your AirPods"] };
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
      return { text: "In this demo I only know a few things; on ClaudeOS, Claude answers anything, and does it. " + TRY };
    }]
  ];
  function answer(text) {
    for (var i = 0; i < INTENTS.length; i++) if (INTENTS[i][0].test(text)) return INTENTS[i][1](text);
    return { text: "" };
  }
  function setScale(s) {
    scale = Math.max(0.6, Math.min(2.2, s));
    store("claudeos-demo-scale", scale);
    layout();
  }

  var turns = 0;
  function ask(text) {
    var uid = "u" + Date.now().toString(36), uts = now(), r = answer(text), id = "a" + (++turns);
    call("user", uid, text, true);
    setTimeout(function () {
      call("aiStart", id);
      setTimeout(function () {
        if (r.act) r.act();
        (r.done || []).forEach(function (d, i) {
          call("done", { id: id + "-" + i, ts: now(), text: d, prompt_id: uid, prompt_ts: uts });
        });
        stream(id, r.text || "");
      }, 600);
    }, 120);
  }
  function stream(id, text) {
    var at = 0;
    (function step() {
      var to = Math.min(text.length, at + 3 + Math.floor(Math.random() * 6));
      call("ai", id, text.slice(at, to));
      at = to;
      if (at < text.length) setTimeout(step, 24);
      else setTimeout(function () { call("aiDone", id); }, 30);
    })();
  }

  function openMessage(id) {
    var m = MESSAGES.filter(function (x) { return x.id === id; })[0];
    if (m && m.unread) { m.unread = false; call("message", withPin(m)); }
    var html = thread(id);
    if (html) show({ html: html });
  }
  function withPin(m) {
    var pins = (store("claudeos-demo-pins") || {}).messages || {}, out = {};
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
  function intro() {
    try { if (window.sessionStorage.getItem("claudeos-demo-seen")) return; } catch (e) {}
    call("accountForm", { id: "intro", title: "ClaudeOS, in your browser", ok: "Try it", text: [
      "This is ClaudeOS's screen. ClaudeOS is Claude running a whole computer: you talk, type or touch, and it does " +
      "things for you. Messages from all your accounts are on the left, what it has done for you on the right.",
      "Here, Claude's answers are written in advance, and the messages and files are made up. " + TRY,
      "Nothing you type here leaves your browser. ClaudeOS is an independent project, not made by Anthropic."] });
  }

  // ------------------------------------------------------------------ what the page says
  function handle(msg) {
    switch (msg.type) {
      case "ready":
        layout();
        call("settings", "", "");
        call("history", HISTORY);
        call("notice", "A demo: Claude's answers here are written in advance, and nothing you type leaves your browser.");
        call("doneList", DONE.map(function (e) {
          var pins = (store("claudeos-demo-pins") || {}).done || {}, o = {};
          for (var k in e) o[k] = e[k];
          if (pins[e.id]) o.pinned = pins[e.id];
          return o;
        }));
        call("messagesList", MESSAGES.map(withPin), ACCOUNTS);
        intro();
        break;
      case "send": ask(String(msg.text || "")); break;
      case "drafting": drafting = !!msg.on; break;
      case "hole": hole = { x: msg.x, y: msg.y, w: msg.w, h: msg.h }; place(); break;
      case "close_content": close(); break;
      case "expand": expanded = !expanded; place(); break;
      case "open_message": openMessage(msg.id); break;
      case "pin": {
        var pins = store("claudeos-demo-pins") || {};
        pins[msg.list] = pins[msg.list] || {};
        if (msg.on) pins[msg.list][msg.id] = now(); else delete pins[msg.list][msg.id];
        store("claudeos-demo-pins", pins);
        break;
      }
      case "goto": call("earlier", EARLIER, msg.id, msg.ts); break;
      case "search": call("searchResults", msg.n, msg.q || "", search(msg.q || ""),
                          search(msg.q || "").length ? "" : "Nothing in the demo's files matches \u201c" + (msg.q || "") + "\u201d.");
        break;
      case "open_file": if (PAGES[msg.path]) show({ html: PAGES[msg.path]() }); break;
      case "text_scale": setScale(scale * (+msg.factor || 1)); break;
      case "add_account":
        call("accountForm", { id: "accounts", title: "Connect an account", ok: "OK", text: [
          "On ClaudeOS, Gmail, Outlook, iCloud Mail, Telegram, WhatsApp, X, Instagram, Facebook, TikTok and LinkedIn " +
          "connect here, and their messages come into Messages, newest first. Sign-ins stay on your computer.",
          "The demo can't connect accounts."] });
        break;
      case "account":
      case "account_cancel":
        if (msg.form === "intro") { try { window.sessionStorage.setItem("claudeos-demo-seen", "1"); } catch (e) {} }
        call("accountForm", null);
        break;
      default: break;            // keys, keyboard: nothing to do in a browser
    }
  }
})();
