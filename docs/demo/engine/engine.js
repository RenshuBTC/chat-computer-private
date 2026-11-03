// The page's side of Chat Computer's own model engine running in the browser (worker.js does the work, in a Web Worker,
// with the engine itself as WebAssembly: engine.wasm, from os/webengine). With it the website's chat answers for real,
// free and with no account: Qwen3.5 0.8B (8 bits a weight), downloaded once from Qwen's own repository and kept by
// the browser, runs on the visitor's own processor, and nothing typed leaves the page.
//
//   ChatComputerEngine.supported()          {ok, why}: whether this browser can run it (WebAssembly's SIMD, workers)
//   ChatComputerEngine.status()             Promise of {name, label, download (bytes), kept, kept_bytes, loaded}
//   ChatComputerEngine.prepare(onProgress)  Promise of the model's settings ({label, layers, context, bytes, ...}),
//                                           once it's loaded: from the browser's copy, or downloaded (onProgress({stage:
//                                           "downloading" | "keeping" | "loading" | "not-kept", done, total}))
//   ChatComputerEngine.ahead(request, onReading)
//                                           reads the request's first message (the system prompt and the tools) ahead
//                                           of the question, while the person types (kept for later visits too)
//   ChatComputerEngine.chat(request, onChunk, {onReading, signal})
//                                           Promise of the whole assistant message ({role, content, reasoning_content,
//                                           tool_calls}); onChunk(delta, end) gets each piece as the engine's program
//                                           streams it ({role}, {content}, {reasoning_content}, {tool_calls: [{index,
//                                           id, type, function: {name, arguments}}]}; the last, {content: ""}, with
//                                           end = {finish_reason, usage}); onReading(done, total) how far the prompt's
//                                           reading has got
//   ChatComputerEngine.stop()               the answer being written stops where it is (its promise resolves)
//   ChatComputerEngine.forget()             Promise: the browser's copy deleted, the engine stopped
//
// A request is OpenAI's chat completions request ({messages, tools, max_tokens, think, temperature...}), as the
// engine's program on a PC reads it.
(function () {
  "use strict";
  var script = document.currentScript;
  var HERE = new URL(".", script && script.src || location.href).href;
  var worker = null, ids = 0, pending = {}, prepared = null, progressTo = [], answering = 0;

  function start() {
    if (worker) return worker;
    worker = new Worker(HERE + "worker.js");
    worker.onmessage = function (e) {
      var m = e.data || {}, p = pending[m.id];
      if (!p) return;
      if (m.type === "progress" && p.on.progress) p.on.progress({ stage: m.stage, done: m.done, total: m.total, why: m.why });
      else if (m.type === "reading" && p.on.reading) p.on.reading(m.done, m.total);
      else if (m.type === "delta" && p.on.delta) p.on.delta(m.delta, m.finish_reason ? { finish_reason: m.finish_reason, usage: m.usage } : undefined);
      else if (m.type === "done") { delete pending[m.id]; p.resolve(m.result); }
      else if (m.type === "error") {
        delete pending[m.id];
        if (m.broken) stopWorker(m.message);                  // (the engine can't go on: a new one next time)
        p.reject(new Error(m.message));
      }
    };
    worker.onerror = function (e) { stopWorker((e && e.message) || "the engine stopped"); };
    return worker;
  }
  function stopWorker(why) {
    if (worker) worker.terminate();
    worker = null;
    prepared = null;
    var was = pending;
    pending = {};
    for (var id in was) was[id].reject(new Error(why));
  }
  function ask(op, data, on) {
    var id = ++ids;
    var p = new Promise(function (resolve, reject) {
      pending[id] = { resolve: resolve, reject: reject, on: on || {} };
      var m = { id: id, op: op };
      for (var k in data || {}) m[k] = data[k];
      start().postMessage(m);
    });
    p.id = id;
    return p;
  }

  // WebAssembly with its SIMD (a tiny module that uses it), workers, and somewhere to keep the model.
  var SIMD = new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0, 1, 5, 1, 96, 0, 1, 123, 3, 2, 1, 0, 10, 22, 1, 20, 0, 253, 12,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 11]);
  function supported() {
    if (typeof WebAssembly !== "object" || !WebAssembly.validate) return { ok: false, why: "this browser doesn't run WebAssembly" };
    if (!WebAssembly.validate(SIMD)) return { ok: false, why: "this browser's WebAssembly hasn't SIMD (it's newer browsers')" };
    if (typeof Worker !== "function") return { ok: false, why: "this browser doesn't run workers" };
    if (!(navigator.storage && navigator.storage.getDirectory) && !self.caches) return { ok: false, why: "this browser keeps no files for pages" };
    return { ok: true, why: "" };
  }

  function prepare(onProgress) {
    if (onProgress) progressTo.push(onProgress);
    if (prepared) return prepared;
    // (Kept for good where the browser allows it: not cleared to make room for other sites'.)
    if (navigator.storage && navigator.storage.persist) navigator.storage.persist().catch(function () {});
    var lock = null;
    try {
      if (navigator.wakeLock && document.visibilityState === "visible") {
        navigator.wakeLock.request("screen").then(function (l) { lock = l; }, function () {});
      }
    } catch (e) {}
    function over() { try { if (lock) lock.release(); } catch (e) {} progressTo = []; }
    prepared = ask("prepare", {}, { progress: function (p) { progressTo.forEach(function (f) { try { f(p); } catch (e) {} }); } })
      .then(function (r) { over(); return r; }, function (e) { over(); prepared = null; throw e; });
    return prepared;
  }

  function chat(request, onChunk, options) {
    options = options || {};
    var p = ask("chat", { request: request }, { delta: onChunk, reading: options.onReading });
    answering = p.id;
    if (options.signal) {
      if (options.signal.aborted) stopAnswer(p.id);
      else options.signal.addEventListener("abort", function () { stopAnswer(p.id); }, { once: true });
    }
    return p.then(function (end) {
      if (answering === p.id) answering = 0;
      return end.message;
    }, function (e) {
      if (answering === p.id) answering = 0;
      throw e;
    });
  }
  function stopAnswer(id) { if (worker && id) worker.postMessage({ op: "stop", of: id }); }

  window.ChatComputerEngine = {
    supported: supported,
    status: function () { return ask("status"); },
    prepare: prepare,
    ahead: function (request, onReading) { return ask("ahead", { request: request }, { reading: onReading }); },
    chat: chat,
    stop: function () { stopAnswer(answering); },
    forget: function () {
      return ask("forget").then(function () { stopWorker("the engine was stopped: its model was deleted"); });
    }
  };
})();
