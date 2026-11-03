// Chat Computer's own model engine (os/model) in a Web Worker of the website's page: the chat answers with it, free and
// with no account, the model running here in the visitor's browser (WebAssembly: engine.wasm, built from os/webengine
// by scripts/build_web_engine.py; no other program or library). engine.js is the page's side of it.
//
// The first time, it gets the model ready: Qwen's published files downloaded from Qwen's own repository on Hugging
// Face, at the revision Chat Computer pins (model.json, made from chatcomputer/brain/models.json), in pieces (Range
// requests: a piece that breaks off or stalls is asked for again from where it stopped), each file checked against its
// pinned SHA-256 as it comes, and the engine's own model file made from them as they come (the same file, byte for
// byte, as the engine's program makes on a PC: 8 bits a weight). Nothing is kept unless every file checked out. The
// browser keeps the model's file (in its own private files, or in its cache where it hasn't those), so later visits
// load it from there with nothing downloaded; and the reading of the conversation's first message (the system prompt
// and the tools), so it isn't read again while they stay the same.
//
// Then it answers a conversation as OpenAI's chat completions API has it, a step at a time: a part of the prompt read,
// or a token written, each step's words, thinking or tool call sent to the page as the engine's program streams them.
//
// Messages in: {id, op}, op one of status, prepare, ahead (read the first message ahead of the question), chat
// (with its request), stop (the answer with that id stops where it is), forget (the kept files are deleted). Out:
// {id, type}, type one of progress, reading, delta, done, error.
"use strict";

var HERE = new URL(".", self.location.href).href;
var STORE = "chatcomputer-engine";       // the browser's store for it: a folder of its private files, or a cache
var PIECE = 32 * 1024 * 1024;            // a download's pieces: one Range request each
var STAGE = 1024 * 1024;                 // what's handed to the module at once
var CHUNK = 64 * 1024 * 1024;            // what's written to or read from the store at once

var W = null, broken = "";               // the module's functions; why it stopped, if it did (a panic: it can't go on)
var manifest = null, info = null;        // the model to get ready (model.json); its settings, once loaded
var stopped = {}, keptMark = null, queue = Promise.resolve();
var asking = 0;                          // questions waiting: reading ahead gives way to them

function post(id, type, data) {
  var m = { id: id, type: type };
  for (var k in data || {}) m[k] = data[k];
  self.postMessage(m);
}

self.onmessage = function (e) {
  var m = e.data || {};
  if (m.op === "stop") { stopped[m.of] = true; return; }      // (at once, between the answer's steps)
  if (m.op === "chat") asking++;
  queue = queue.then(function () { return handle(m); }).then(function (r) {
    post(m.id, "done", { result: r || {} });
  }, function (err) {
    post(m.id, "error", { message: String(err && err.message || err), broken: !!broken });
  });
};

function handle(m) {
  if (m.op === "status") return status();
  if (m.op === "prepare") return prepare(m.id);
  if (m.op === "ahead") return ahead(m.id, m.request || {});
  if (m.op === "chat") { asking--; return chat(m.id, m.request || {}); }
  if (m.op === "forget") return forget();
  return Promise.reject(new Error("no such thing to do: " + m.op));
}

// ------------------------------------------------------------------ the module
function engine() {
  if (broken) return Promise.reject(new Error(broken));
  if (W) return Promise.resolve(W);
  var url = HERE + "engine.wasm";
  var made = WebAssembly.instantiateStreaming ? WebAssembly.instantiateStreaming(fetch(url), {}).catch(function () {
    return whole(url);                    // (a server that doesn't say it's WebAssembly)
  }) : whole(url);
  function whole(u) {
    return fetch(u).then(function (r) {
      if (!r.ok) throw new Error("the engine's file didn't come (" + r.status + ")");
      return r.arrayBuffer();
    }).then(function (b) { return WebAssembly.instantiate(b, {}); });
  }
  return made.then(function (r) {
    W = r.instance.exports;
    W.cc_init();
    return W;
  });
}
// The module's memory: a view of `len` bytes at `ptr` (made again after each call: the memory may have grown).
function mem(ptr, len) { return new Uint8Array(W.memory.buffer, ptr >>> 0, len); }
function outBytes() { return mem(W.cc_out_ptr(), W.cc_out_len()); }
function outText() { return new TextDecoder().decode(outBytes()); }
function outJson() { var t = outText(); return t ? JSON.parse(t) : {}; }
// Calls one of the module's functions: what it gives back (JSON), or its reason as an error. A panic stops the
// module for good (the page then starts a new worker): what it said is the error.
function call(name) {
  var args = Array.prototype.slice.call(arguments, 1), r;
  try {
    r = W[name].apply(null, args);
  } catch (e) {
    if (e instanceof WebAssembly.RuntimeError) {
      try { W.cc_panic(); broken = "the engine stopped: " + (outText() || e.message); } catch (x) { broken = "the engine stopped: " + e.message; }
      throw new Error(broken);
    }
    throw e;
  }
  if (r < 0) throw new Error(outJson().error || "the engine couldn't do that");
  return r;
}
// Bytes put in the module's memory for a call: f(ptr, len), the room given back after.
function withBytes(bytes, f) {
  var p = W.cc_alloc(bytes.length) >>> 0;
  if (!p) throw noMemory("the engine");
  mem(p, bytes.length).set(bytes);
  try { return f(p, bytes.length); } finally { W.cc_free(p, bytes.length); }
}
function text(s) { return new TextEncoder().encode(s); }
// (Short of memory: nothing's wrong with what's kept, which stays.)
function noMemory(what) {
  var e = new Error("this browser has no more memory to give " + what + " here");
  e.memory = true;
  return e;
}

// ------------------------------------------------------------------ the browser's store
// Its private files (the Origin Private File System, written and read in place from a worker), or its cache.
var folder = null;
function files() {
  if (folder) return Promise.resolve(folder);
  if (!navigator.storage || !navigator.storage.getDirectory || typeof FileSystemFileHandle === "undefined" ||
      !FileSystemFileHandle.prototype.createSyncAccessHandle) return Promise.resolve(null);
  return navigator.storage.getDirectory().then(function (root) { return root.getDirectoryHandle(STORE, { create: true }); })
    .then(function (d) { folder = d; return d; }, function () { return null; });
}
function cache() { return self.caches ? caches.open(STORE) : Promise.reject(new Error("this browser keeps no files for pages")); }
function cacheKey(name) { return HERE + "kept/" + name; }

// A kept file's size, or -1 if there's none.
function keptSize(name) {
  return files().then(function (d) {
    if (d) {
      return d.getFileHandle(name).then(function (h) { return h.getFile(); }).then(function (f) { return f.size; }, function () { return -1; });
    }
    return cache().then(function (c) { return c.match(cacheKey(name)); }).then(function (r) {
      return r ? r.blob().then(function (b) { return b.size; }) : -1;
    }, function () { return -1; });
  });
}
// A small kept file's bytes (null if there's none).
function readKept(name) {
  return files().then(function (d) {
    if (d) {
      return d.getFileHandle(name).then(function (h) { return h.getFile(); })
        .then(function (f) { return f.arrayBuffer(); }).then(function (b) { return new Uint8Array(b); }, function () { return null; });
    }
    return cache().then(function (c) { return c.match(cacheKey(name)); }).then(function (r) {
      return r ? r.arrayBuffer().then(function (b) { return new Uint8Array(b); }) : null;
    }, function () { return null; });
  });
}
// A big kept file read straight into the module's memory: into(size) gives where (0: no room); report(done, total).
function readKeptInto(name, size, into, report) {
  return files().then(function (d) {
    if (d) {
      return d.getFileHandle(name).then(function (h) { return h.createSyncAccessHandle(); }).then(function (h) {
        try {
          if (h.getSize() !== size) throw new Error("the kept model isn't whole");
          var p = into(size);
          if (!p) throw noMemory("the model");
          for (var at = 0; at < size;) {
            var n = h.read(mem(p + at, Math.min(CHUNK, size - at)), { at: at });
            if (n <= 0) throw new Error("the kept model couldn't be read");
            at += n;
            report(at, size);
          }
        } finally { h.close(); }
      });
    }
    return cache().then(function (c) { return c.match(cacheKey(name)); }).then(function (r) {
      if (!r || !r.body) throw new Error("the kept model isn't there");
      var p = into(size), at = 0, reader = r.body.getReader();
      if (!p) throw noMemory("the model");
      return (function more() {
        return reader.read().then(function (x) {
          if (x.done) { if (at !== size) throw new Error("the kept model isn't whole"); return; }
          if (at + x.value.length > size) throw new Error("the kept model isn't whole");
          mem(p + at, x.value.length).set(x.value);
          at += x.value.length;
          report(at, size);
          return more();
        });
      })();
    });
  });
}
// Keeps bytes (a view: of the module's memory, say) under a name, replacing what was there.
function keep(name, view, report) {
  return files().then(function (d) {
    if (d) {
      return d.getFileHandle(name, { create: true }).then(function (h) { return h.createSyncAccessHandle(); }).then(function (h) {
        try {
          h.truncate(0);
          for (var at = 0; at < view.length;) {
            at += h.write(view.subarray(at, Math.min(view.length, at + CHUNK)), { at: at });
            if (report) report(at, view.length);
          }
          h.flush();
        } finally { h.close(); }
      });
    }
    return cache().then(function (c) { return c.put(cacheKey(name), new Response(new Blob([view]))); });
  });
}
function unkeep(name) {
  return files().then(function (d) {
    if (d) return d.removeEntry(name).catch(function () {});
    return cache().then(function (c) { return c.delete(cacheKey(name)); }).catch(function () {});
  });
}

// ------------------------------------------------------------------ the model to get ready
function model() {
  if (manifest) return Promise.resolve(manifest);
  return fetch(HERE + "model.json").then(function (r) {
    if (!r.ok) throw new Error("the model's description didn't come (" + r.status + ")");
    return r.json();
  }).then(function (m) { manifest = m; return m; });
}
// The kept files' names: the model's own file, what says it's whole (written last), and the first message's reading.
function names(m) {
  var key = m.name + "-" + m.revision.slice(0, 12) + "-" + m.bits + "bit";
  return { model: key + ".ccm", whole: key + ".json", anchor: key + ".anchor" };
}
function fileUrl(m, f) { return (m.base || "https://huggingface.co") + "/" + m.repo + "/resolve/" + m.revision + "/" + encodeURIComponent(f.name); }
function downloadBytes(m) { return m.files.reduce(function (a, f) { return a + f.size; }, 0); }

// Whether the model is kept here (and how big its file is), and how much its download is.
function status() {
  return model().then(function (m) {
    var n = names(m);
    return Promise.all([readKept(n.whole), keptSize(n.model)]).then(function (r) {
      var whole = null;
      try { whole = r[0] && JSON.parse(new TextDecoder().decode(r[0])); } catch (e) {}
      var kept = !!whole && whole.size === r[1];
      return { name: m.name, label: m.label, download: downloadBytes(m), kept: kept, kept_bytes: kept ? r[1] : 0, loaded: !!info };
    });
  });
}

// Gets the model ready: loaded from what's kept, or downloaded, checked, made into the engine's file and kept. One tab
// at a time (the others wait, then find it kept).
function prepare(id) {
  if (info) return Promise.resolve(info);
  var go = function () {
    return Promise.all([engine(), model()]).then(function (r) {
      var m = r[1], n = names(m);
      return Promise.all([status(), refusal(m)]).then(function (k) {
        var s = k[0], taken = k[1];
        if (s.kept) {
          return load(id, m, n, s.kept_bytes, taken).catch(function (err) {
            if (broken || err.memory) throw err;
            // (A kept file this engine can't use, or a half-kept one: made again.)
            return Promise.all([unkeep(n.whole), unkeep(n.model), unkeep(n.anchor)]).then(function () { return fresh(id, m, n, taken); });
          });
        }
        return fresh(id, m, n, taken);
      });
    });
  };
  return (navigator.locks ? navigator.locks.request(STORE, go) : go()).then(function (r) {
    info = r;
    return r;
  });
}

// The kept model, read into the module's memory and loaded (with its refusal direction, if it has one); the first
// message's reading too, if it's kept.
function load(id, m, n, size, taken) {
  var last = 0;
  return readKeptInto(n.model, size, function (len) { return W.cc_model_alloc(len) >>> 0; }, function (done, total) {
    if (Date.now() - last > 200 || done === total) { last = Date.now(); post(id, "progress", { stage: "loading", done: done, total: total }); }
  }).then(function () { return loaded(m, n, taken); });
}

// The model's refusal direction, where Chat Computer gives one beside the engine (as the program finds it:
// chat-computer-model find-refusal), checked against its pinned SHA-256 before anything else is done: taken out as
// the model runs.
function refusal(m) {
  if (!m.refusal) return Promise.resolve(null);
  return fetch(HERE + m.refusal.name).then(function (r) {
    if (!r.ok) throw new Error("the model's refusal file didn't come (" + r.status + ")");
    return r.arrayBuffer();
  }).then(function (b) {
    var bytes = new Uint8Array(b);
    if (bytes.length !== m.refusal.size || sha256(bytes) !== m.refusal.sha256) throw new Error("the model's refusal file isn't the one pinned");
    return bytes;
  });
}
function sha256(bytes) {
  var h = W.cc_hash_new();
  withBytes(bytes, function (p, len) { call("cc_hash_update", h, p, len); });
  call("cc_hash_finish", h);
  return outText();
}

function loaded(m, n, refusalBytes) {
  if (refusalBytes) withBytes(refusalBytes, function (p, len) { call("cc_load", p, len); });
  else call("cc_load", 0, 0);
  var settings = outJson();
  settings.name = m.name;
  settings.label = m.label;
  return readKept(n.anchor).then(function (a) {
    if (a && a.length) {
      if (withBytes(a, function (p, len) { return call("cc_anchor_import", p, len); }) === 0) {
        call("cc_anchor_mark");
        keptMark = outText();
      }
    }
    return settings;
  });
}

// Downloaded, checked, made, kept and loaded.
function fresh(id, m, n, taken) {
  var total = downloadBytes(m), got = 0, last = 0;
  function progress(stage, done, all) {
    if (Date.now() - last > 200 || done === all) { last = Date.now(); post(id, "progress", { stage: stage, done: done, total: all }); }
  }
  function counted(k) { got += k; progress("downloading", got, total); }
  progress("downloading", 0, total);
  return room(m).then(function () {
    var ccm = m.files.filter(function (f) { return /\.ccm$/.test(f.name); })[0];
    return ccm ? fetchModelFile(m, ccm, counted) : fetchAndConvert(m, counted);
  }).then(function () {
    var len = W.cc_model_len(), ptr = W.cc_model_ptr() >>> 0;
    progress("keeping", 0, len);
    // (Kept, then marked whole: a file without its mark is one that broke off, and is made again.)
    return keep(n.model, mem(ptr, len), function (done) { progress("keeping", done, len); })
      .then(function () {
        return keep(n.whole, text(JSON.stringify({ name: m.name, revision: m.revision, bits: m.bits, size: len, kept: new Date().toISOString() })));
      }).catch(function (err) {
        // (No room for it: it runs this time, and is downloaded again next time.)
        post(id, "progress", { stage: "not-kept", done: 0, total: 0, why: String(err && err.message || err) });
        return Promise.all([unkeep(n.model), unkeep(n.whole)]);
      });
  }).then(function () { return loaded(m, n, taken); });
}

// Whether this device has the memory to run the model, and the browser room to keep its file (its own estimate; the
// download itself isn't kept): said before anything is downloaded.
function room(m) {
  var need = m.files.filter(function (f) { return /\.(safetensors|ccm)$/.test(f.name); })
    .reduce(function (a, f) { return a + (/\.ccm$/.test(f.name) ? f.size : f.size * (m.bits + 0.5) / 16); }, 0);
  try {
    // (The model's file, and room to run it: a browser that can't give a page that much says so here.)
    new WebAssembly.Memory({ initial: Math.ceil((need + 256e6) / 65536) });
  } catch (e) {
    return Promise.reject(new Error("this browser can't give the model the " + gb(need + 256e6) + " of memory it needs"));
  }
  if (!navigator.storage || !navigator.storage.estimate) return Promise.resolve();
  return navigator.storage.estimate().then(function (e) {
    if (e.quota && e.quota - (e.usage || 0) < need) {
      throw new Error("this browser has room for " + gb(e.quota - (e.usage || 0)) + " of this site's files, and the model needs " + gb(need));
    }
  }, function () {});
}
function gb(b) { return (b / 1e9).toFixed(1) + " GB"; }

// A file already in the engine's format (Chat Computer's tests give one), straight into the module's memory.
function fetchModelFile(m, f, counted) {
  var p = W.cc_model_alloc(f.size) >>> 0, h = W.cc_hash_new();
  if (!p) return Promise.reject(noMemory("the model"));
  return stream(fileUrl(m, f), 0, f.size, function (chunk, at) {
    mem(p + at, chunk.length).set(chunk);
    call("cc_hash_update", h, p + at, chunk.length);
    counted(chunk.length);
  }).then(function () { checkHash(h, f); });
}

function checkHash(h, f) {
  call("cc_hash_finish", h);
  if (outText() !== f.sha256) throw new Error(f.name + " isn't the file Chat Computer pins (its SHA-256 differs): nothing was kept");
}

// Qwen's published files: the small ones whole, each weights file's header, the plan, then each weights file's bytes
// in order, each checked as it comes and made into the model's file as it comes.
function fetchAndConvert(m, counted) {
  var small = m.files.filter(function (f) { return !/\.safetensors$/.test(f.name); });
  var weights = m.files.filter(function (f) { return /\.safetensors$/.test(f.name); })
    .sort(function (a, b) { return a.name < b.name ? -1 : a.name > b.name ? 1 : 0; });
  if (!weights.length) return Promise.reject(new Error("the model has no weights files"));
  var stage = W.cc_alloc(STAGE) >>> 0;
  if (!stage) return Promise.reject(noMemory("the engine"));
  call("cc_convert_new", m.bits);
  // Bytes from the download, through the module: hashed, and (for a weights file) fed to the converter.
  function take(h, file, chunk, at) {
    for (var i = 0; i < chunk.length; i += STAGE) {
      var part = chunk.subarray(i, Math.min(chunk.length, i + STAGE));
      mem(stage, part.length).set(part);
      call("cc_hash_update", h, stage, part.length);
      if (file >= 0) call("cc_convert_feed", file, at + i, stage, part.length);
    }
    counted(chunk.length);
  }
  return small.reduce(function (p, f) {
    return p.then(function () {
      var bytes = new Uint8Array(f.size), h = W.cc_hash_new();
      return stream(fileUrl(m, f), 0, f.size, function (chunk, at) { bytes.set(chunk, at); take(h, -1, chunk, at); }).then(function () {
        checkHash(h, f);
        withBytes(text(f.name), function (np, nl) {
          withBytes(bytes, function (p2, l2) { call("cc_convert_file", np, nl, p2, l2); });
        });
      });
    });
  }, Promise.resolve()).then(function () {
    // Each weights file's header (its first 8 bytes say how long it is), for the plan.
    return weights.reduce(function (p, f) {
      return p.then(function (ids) {
        var url = fileUrl(m, f), head = new Uint8Array(8);
        return stream(url, 0, 8, function (c, at) { head.set(c, at); }).then(function () {
          var len = new DataView(head.buffer).getUint32(0, true) + 0x100000000 * new DataView(head.buffer).getUint32(4, true);
          if (len > 100 * 1024 * 1024 || 8 + len > f.size) throw new Error(f.name + " isn't a weights file");
          var header = new Uint8Array(len);
          return stream(url, 8, 8 + len, function (c, at) { header.set(c, at - 8); }).then(function () {
            return withBytes(text(f.name), function (np, nl) {
              return withBytes(header, function (hp, hl) { return ids.concat([call("cc_convert_weights", np, nl, hp, hl)]); });
            });
          });
        });
      });
    }, Promise.resolve([]));
  }).then(function (ids) {
    call("cc_convert_plan");
    return weights.reduce(function (p, f, k) {
      return p.then(function () {
        var h = W.cc_hash_new();
        return stream(fileUrl(m, f), 0, f.size, function (chunk, at) { take(h, ids[k], chunk, at); }).then(function () { checkHash(h, f); });
      });
    }, Promise.resolve());
  }).then(function () {
    W.cc_free(stage, STAGE);
    call("cc_convert_finish");
  }, function (err) {
    if (!broken) W.cc_free(stage, STAGE);
    throw err;
  });
}

// ------------------------------------------------------------------ downloading
function sleep(ms) { return new Promise(function (go) { setTimeout(go, ms); }); }
function online() {
  return navigator.onLine === false ? new Promise(function (go) { self.addEventListener("online", go, { once: true }); }) : Promise.resolve();
}
// Bytes [from, to) of a file, a piece at a time, each chunk to take(chunk, its offset) as it comes. A piece that breaks
// off or stalls for 20 seconds is asked for again from where it stopped (up to 8 times in a row, waiting longer each
// time, and for the network to be back).
function stream(url, from, to, take) {
  var at = from, tries = 0;
  function next() {
    if (at >= to) return Promise.resolve();
    var end = Math.min(to, at + PIECE);
    return piece(url, at, end, function (c) { take(c, at); at += c.length; }).then(function () {
      tries = 0;
      return next();
    }, function (err) {
      if (err && err.fatal) throw err;
      if (++tries > 8) throw new Error("the download kept stopping (" + (err && err.message || err) + ")");
      return online().then(function () { return sleep(Math.min(30000, 1000 * Math.pow(2, tries - 1))); }).then(next);
    });
  }
  return next();
}
function piece(url, from, end, take) {
  var stop = new AbortController(), last = Date.now();
  var timer = setInterval(function () { if (Date.now() - last > 20000) stop.abort(); }, 2000);
  return fetch(url, { headers: { Range: "bytes=" + from + "-" + (end - 1) }, signal: stop.signal, cache: "no-store" }).then(function (res) {
    if (res.status !== 206 || !res.body) {
      var err = new Error("Hugging Face said " + res.status + " for " + url.split("/").pop());
      err.fatal = res.status === 404 || res.status === 403 || res.status === 401 || res.status === 200;
      throw err;
    }
    var range = /bytes (\d+)-(\d+)\//.exec(res.headers.get("Content-Range") || "");
    if (range && +range[1] !== from) throw new Error("the server sent another part");
    var reader = res.body.getReader(), got = 0, want = end - from;
    return (function more() {
      return reader.read().then(function (r) {
        if (r.done) {
          if (got !== want) throw new Error("a piece came short");
          return;
        }
        last = Date.now();
        var c = r.value.length > want - got ? r.value.subarray(0, want - got) : r.value;
        got += c.length;
        // (What's done with the bytes going wrong isn't the network's doing: not asked for again.)
        try { if (c.length) take(c); } catch (e) { e.fatal = true; throw e; }
        return got < want ? more() : reader.cancel().catch(function () {});
      });
    })();
  }).then(function (v) { clearInterval(timer); return v; }, function (e) { clearInterval(timer); throw e; });
}

// ------------------------------------------------------------------ answering
// Lets the worker's other messages in (a stop) between steps, without the delay a timer has.
var yielded = 0;
function breathe() {
  if (Date.now() - yielded < 50) return Promise.resolve();
  return new Promise(function (go) {
    var ch = new MessageChannel();
    ch.port1.onmessage = function () { yielded = Date.now(); go(); };
    ch.port2.postMessage(null);
  });
}
// How many of the prompt's tokens a step reads: more while steps are quick, fewer when they're slow, so a stop is
// heard within a moment and the reading's progress moves.
function suited(read, ms) { return ms < 60 ? Math.min(64, read * 2) : ms > 250 ? Math.max(1, read >> 1) : read; }

function requestBytes(r) { return text(JSON.stringify(r)); }

// Reads the first message ahead of the question; kept for later visits once read. A question asked meanwhile takes
// over at once: what's read so far stays read, and the question's own reading goes on from there.
function ahead(id, request) {
  if (!info) return Promise.reject(new Error("the model isn't ready"));
  withBytes(requestBytes(request), function (p, len) { call("cc_ahead", p, len); });
  var tokens = outJson().tokens, read = 8, done = 0, over = false;
  function more() {
    if (asking > 0) return Promise.resolve();
    var t = Date.now(), r = call("cc_ahead_step", read);
    done = r ? Math.min(tokens, done + read) : tokens;
    post(id, "reading", { done: done, total: tokens });
    read = suited(read, Date.now() - t);
    if (!r) { over = true; return keepAnchor(); }
    return breathe().then(more);
  }
  return more().then(function () { return { tokens: tokens, read: over }; });
}

// The first message's reading, kept when it's new (so the next visit starts from it).
function keepAnchor() {
  call("cc_anchor_mark");
  var mark = outText();
  if (!mark || mark === keptMark) return Promise.resolve();
  call("cc_anchor_export");
  var n = names(manifest);
  return keep(n.anchor, outBytes()).then(function () { keptMark = mark; }, function () {});
}

function chat(id, request) {
  if (!info) return Promise.reject(new Error("the model isn't ready"));
  withBytes(requestBytes(request), function (p, len) { call("cc_chat", p, len); });
  var begun = outJson(), read = 8, end = null;
  post(id, "delta", { delta: { role: "assistant" } });
  if (begun.reading) post(id, "reading", { done: 0, total: begun.prompt_tokens });
  function more() {
    var r, t = Date.now();
    r = stopped[id] ? call("cc_stop") : call("cc_step", read);
    var o = outJson();
    if (o.read) {
      read = suited(read, Date.now() - t);
      post(id, "reading", { done: o.read[0], total: o.read[1] });
    }
    if (o.delta) post(id, "delta", { delta: o.delta });
    if (r === 0) { end = o.end; return Promise.resolve(); }
    return breathe().then(more);
  }
  return more().then(function () {
    delete stopped[id];
    post(id, "delta", { delta: { content: "" }, finish_reason: end.finish_reason, usage: end.usage });
    return keepAnchor().then(function () { return end; });
  }, function (err) {
    delete stopped[id];
    if (!broken) { try { call("cc_stop"); } catch (x) {} }     // (the turn ends here, so the next can begin)
    throw err;
  });
}

// ------------------------------------------------------------------ forgetting
function forget() {
  return model().then(function (m) {
    var n = names(m);
    return Promise.all([unkeep(n.whole), unkeep(n.model), unkeep(n.anchor)]);
  }).then(function () { return {}; });
}
