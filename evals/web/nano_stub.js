// A stand-in for Chrome's built-in AI (LanguageModel, Chrome's Prompt API), for evals/web/run.js --nano. It answers
// the cases' open questions the way the page asks Gemini Nano to (JSON, per its NANO_SCHEMA), so the eval tests the
// page's side: that the answer shows, its action is done, Actions is told, and a failure falls back to the
// written answers. It says nothing about how well Gemini Nano itself answers: only real Chrome can show that.
//
// Like Gemini Nano is asked to, it says what to look up for something recent or that it doesn't know (action
// search), and then answers from what the page found (a prompt that starts "The question:").
//
// What the page sent is kept in window.__nano (the session's options, each prompt and its options, what was appended).
(function () {
  var seen = window.__nano = { created: [], prompts: [], appended: [] };
  var LOOK = [[/prime minister/i, "Prime Minister of Singapore"], [/world cup/i, "2026 World Cup winner"],
              [/marina bay sands/i, "Marina Bay Sands architect"], [/population/i, "Singapore population"],
              [/\bhaze\b|\bair quality\b/i, "Singapore haze"], [/headlines|news|yesterday/i, "news"],
              [/worst one/i, "Southeast Asian haze worst"]];
  var asked = "";          // what "look it up" is about: the question before it
  function found(text) {
    var first = /\n1\. ([^\n]*)/.exec(text);
    if (!first) return { reply: "I looked, but found nothing on that.", action: "none" };
    var what = first[1].replace(/^[^:]*: /, "");
    return { reply: "According to " + (/Wikipedia/.test(first[1]) ? "Wikipedia" : "Open-Meteo") + ": " + what.split(/(?<=\.)\s/)[0] +
             (/\n2\. /.test(text) ? " " + /\n2\. ([^\n]*)/.exec(text)[1].replace(/^[^:]*: /, "").split(/(?<=\.)\s/)[0] : ""), action: "none" };
  }
  var CANNED = [
    [/capital of australia/i, { reply: "Canberra is the capital of Australia.", action: "none" }],
    [/recipe|chicken rice/i, { reply: "Here's a recipe for Hainanese chicken rice, on the screen.", action: "show_page",
      title: "Chicken rice", markdown: "## Ingredients\n- 1 whole chicken\n- 2 cups jasmine rice\n- Ginger, garlic, pandan\n\n" +
      "## Steps\n1. Poach the chicken with ginger for 40 minutes.\n2. Cook the rice in the stock with garlic.\n3. Serve with chilli sauce." }],
    [/quantum/i, { reply: "Quantum computers use qubits, which can be 0 and 1 at once; there's more on the screen.", action: "show_page",
      title: "Quantum computing, simply", markdown: "A normal computer's bits are 0 or 1. A quantum computer's **qubits** can be " +
      "in a mix of both (superposition), and linked to each other (entanglement), which lets it try many answers at once " +
      "for some problems." }],
    [/email.*boss|boss.*friday/i, { reply: "Here's a draft asking your boss for Friday off.", action: "show_page",
      title: "Asking for Friday off", markdown: "Hi Sam,\n\nCould I take this Friday off? My work is up to date, and Alex " +
      "can cover anything urgent.\n\nThanks,\n[Your name]" }],
    [/poem.*sea|sea.*poem/i, { reply: "Here's a short poem about the sea.", action: "show_page", title: "The sea",
      markdown: "The tide comes in on silver feet,\nthe waves fold over, soft and neat;\nthe shore keeps every word they say,\n" +
      "and gives it back another day." }],
    [/thank you in japanese/i, { reply: "Thank you in Japanese is arigatou (ありがとう), or arigatou gozaimasu to be polite.", action: "none" }],
    [/trip to bali/i, { reply: "Here's a 3-day plan for Bali.", action: "show_page", title: "3 days in Bali",
      markdown: "## Day 1\nUbud: rice terraces and the Monkey Forest.\n\n## Day 2\nTemples: Tanah Lot at sunset.\n\n## Day 3\nThe beach at Seminyak." }],
    [/sleep/i, { reply: "Keep to the same bedtime, stop screens an hour before bed, and skip caffeine after lunch.", action: "none" }],
    [/python.*revers|revers.*python/i, { reply: "Here's the function: it uses a slice to reverse the string.", action: "show_page",
      title: "Reverse a string", markdown: "```python\ndef reverse(s):\n    return s[::-1]\n```" }]
  ];
  function answer(text) {
    if (/^The question:/.test(text)) return found(text);
    var about = /^look it up\b/i.test(text) ? asked : text;
    if (!/^look it up\b/i.test(text)) asked = text;
    for (var j = 0; j < LOOK.length; j++) if (LOOK[j][0].test(about)) return { reply: "Let me look.", action: "search", query: LOOK[j][1] };
    for (var i = 0; i < CANNED.length; i++) if (CANNED[i][0].test(text)) return CANNED[i][1];
    if (/^[\W\d_]*[a-z]{0,8}[\W\d_]*$/i.test(text)) {
      return { reply: "I'm not sure what you mean. Try asking me to play music or show a map.", action: "none" };
    }
    return { reply: "I'm not sure about that. Try asking me to play music or show a map.", action: "none" };
  }
  var session = {
    prompt: function (text, opts) {
      seen.prompts.push({ text: text, constrained: !!(opts && opts.responseConstraint) });
      return new Promise(function (ok) { setTimeout(function () { ok(JSON.stringify(answer(text))); }, 400); });
    },
    append: function (messages) { seen.appended.push(messages); return Promise.resolve(); },
    destroy: function () {}
  };
  window.LanguageModel = {
    availability: function () { return Promise.resolve("available"); },
    create: function (opts) {
      seen.created.push({ system: !!(opts && opts.initialPrompts && opts.initialPrompts[0] && opts.initialPrompts[0].role === "system"),
                          inputs: opts && opts.expectedInputs, outputs: opts && opts.expectedOutputs });
      return new Promise(function (ok) { setTimeout(function () { ok(session); }, 50); });
    }
  };
})();
