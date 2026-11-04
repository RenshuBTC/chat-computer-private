"""Pass/fail checks for the website's answers (evals/web/run.js results), one per case, written from the error
analysis of the scripted baseline (labels/scripted-baseline.json): does the answer (and what opened between the
sidebars) do what a first-time visitor asked, or say honestly what this page can't do?

And checks of the failures the error analysis of 4 November 2026 found (labels/scripted-v3.json, the written
answers; labels/web-model-v1.json, Chat Computer's model in the browser), each pass/fail for every case:

  catch_all    - "I can't answer that yet" (or "I can't <do that> yet") where the page could have answered, or a
                 model was there to (the cases only a model can answer are left out when no model was there)
  stale_step   - the way to real answers given as the app's model only ("Use this computer's model"), on the website
  empty        - an answer with no words (the model's turn ended on tool calls)
  round_cap    - the model went round its tools as often as it may (8 rounds), or asked for the same tool with the same
                 input twice in one turn
  bad_input    - a tool told the model its input was wrong ("That box isn't...", "There's no tool called...")
  stray_tool   - a tool the words asked for nothing of: the weather, a map, a timer, YouTube or closing, with none of
                 their words in the question (weather for "hello", a timer for "play music")
  time_stamp   - the model answered the time stamp's place rather than the question: Singapore (the visitor's time
                 zone) in a tool call or the answer, unasked and unneeded

    python -m evals.web.evaluate RESULTS.json [--labels evals/web/labels/scripted-v3.json]

With --labels it says where the checks and the hand labels disagree (how far to trust the checks): the true
positive rate (fails the checks catch) and true negative rate (passes they leave alone), as the evals FAQ has it."""
from __future__ import annotations

import argparse
import json
import re
from pathlib import Path
from typing import Any, Callable

HERE = Path(__file__).resolve().parent
R = dict[str, Any]


def said(r: R, pattern: str) -> bool:
    return re.search(pattern, r.get("answer") or "", re.I) is not None


def shown(r: R, pattern: str) -> bool:
    """What opened between the sidebars: its address, title or text."""
    return bool(r.get("open")) and re.search(pattern, " ".join(str(r.get(k) or "") for k in ("src", "title", "body")),
                                            re.I) is not None


def did(r: R, pattern: str) -> bool:
    return any(re.search(pattern, d, re.I) for d in (r.get("done") or [])[-max(1, r.get("done_added") or 1):]) \
        if r.get("done_added") else False


# The page can't look things up (no search, no YouTube search) without its model: saying so, and what it can do
# instead, is a pass for a request it can't do.
CANT = r"can't|cannot|isn't able|not able|only (know|show|do)|unable|no way to|needs? (its|my|the) model|(on|in) (a|the real) chat computer|use this computer's model|chat computer app"
MUSIC = r"piano|music|chords|playing|lofi|jazz"
# (Or, with no model here yet, the buttons that get one: the answer's own words then say only that one's needed.)
NOT_STUCK = r"try|use this computer's model|you can|say|type|sign in to your ai|download my model|start my model"
NO_AI = r"without an ai|written in advance"     # the page's honest note when only an AI could answer
# The buttons under an answer that needs a model, where none is here yet (getting one: mine, or an AI of yours).
GET_ONE = r"sign in to your ai|download my model|start my model"


# Saying it can't go on the internet: what the browser's own AI said to every question that needed it, before the
# page looked things up for it (the visitor's own conversation: "I am not connected to the internet").
NO_NET = (r"not connected to the internet|no internet|(don't|do not) have (internet|access to (the )?internet|real-time)|"
          r"(can't|cannot|can not|unable to) (look|search|browse|access|go on)|not able to (look|search|browse)|"
          r"(can't|cannot) (get|find) (real-time|current|live)|offline")


def answers(r: R) -> list[str]:
    return [a or "" for a in (r.get("answers") or [r.get("answer") or ""])]


def online(r: R) -> bool:
    """None of its answers says it can't go on the internet."""
    return not any(re.search(NO_NET, a, re.I) for a in answers(r))


def turn(r: R, i: int, pattern: str) -> bool:
    a = answers(r)
    return -len(a) <= i < len(a) and re.search(pattern, a[i], re.I) is not None


def dead_end(r: R) -> bool:
    """The scripted page's old catch-all ("In this demo I only know a few things..."): it answers nothing asked."""
    return said(r, r"only know a few things")


DEAD_END_OK = {"gibberish", "question-mark"}      # nothing was asked: the catch-all is a fair answer


def src(r: R, pattern: str) -> bool:
    """What opened is that site itself (its address or title), not a page that mentions it."""
    return bool(r.get("open")) and re.search(pattern, f"{r.get('src') or ''} {r.get('title') or ''}", re.I) is not None


# Where the stand-in's places are (evals/web/run.js), to check a map shows the place asked for.
WHERE = {"tokyo": (139.69, 35.69), "paris": (2.35, 48.85), "london": (-0.13, 51.51), "berlin": (13.41, 52.52)}


def map_of(r: R, place: str) -> bool:
    """A map is open with the place inside its box, and the box no wider than a few degrees (not a continent, nor a
    strip with the place at its edge)."""
    m = re.search(r"bbox=([-\d.]+)%2C([-\d.]+)%2C([-\d.]+)%2C([-\d.]+)", r.get("src") or "")
    if not (r.get("open") and m):
        return False
    w, so, e, n = map(float, m.groups())
    x, y = WHERE[place]
    inside = w < x < e and so < y < n
    middle = abs((w + e) / 2 - x) < (e - w) / 3 and abs((so + n) / 2 - y) < (n - so) / 3
    return inside and middle and (e - w) < 4 and (n - so) < 4


def written(r: R, pattern: str) -> bool:
    """Shown as a written page (not a site's search, a video or a map, whose title only repeats the question)."""
    return shown(r, pattern) and not re.search(r"youtube|openstreetmap|google\.com/maps", f"{r.get('src') or ''} {r.get('title') or ''}", re.I)


def music(r: R) -> bool:
    return (did(r, r"play") or shown(r, r"piano|music|youtube")) and not said(r, r"\bcan't\b.*\bplay\b")


DONE: dict[str, Callable[[R], bool]] = {
    "what-is-this": lambda r: said(r, r"chat computer") and said(r, r"talk|type|touch") and said(r, r"try|ask me|" + GET_ONE),
    "how-work": lambda r: said(r, r"talk|type|touch") and said(r, r"try|do it|does it"),
    "what-can-do": lambda r: said(r, r"music|maps?|messages|files|websites") and said(r, r"try|ask me|" + GET_ONE),
    "is-this-an-os": lambda r: said(r, r"\b(yes|operating system|runs the whole computer|instead of (a )?desktop)\b")
                               and said(r, r"linux|windows|mac"),
    "hello": lambda r: said(r, r"\b(hi|hello|hey)\b") and said(r, NOT_STUCK),
    "hi-there": lambda r: said(r, r"\b(hi|hello|hey)\b") and said(r, NOT_STUCK),
    "how-are-you": lambda r: said(r, r"\b(good|great|well|fine|thanks)\b") and not said(r, r"only know a few things"),
    "thanks": lambda r: said(r, r"welcome|any ?time|glad|pleasure|no problem"),
    "play-music": music,
    "play-piano": lambda r: music(r),
    "play-taylor": lambda r: src(r, r"youtube.*taylor|taylor") or (said(r, CANT) and said(r, r"youtube|search")),
    "play-lofi-typo": music,
    "im-bored": lambda r: r.get("open") or said(r, r"\b(try|how about|want me to|music|game|map)\b"),
    "relax": lambda r: music(r) or said(r, r"\b(breath|calm|music|piano)\b"),
    "youtube-video": lambda r: src(r, r"youtube.*(cat|watch)") or (said(r, CANT) and said(r, r"youtube|search")),
    "map-tokyo": lambda r: map_of(r, "tokyo") and said(r, r"tokyo"),
    "map-short": lambda r: map_of(r, "paris") and said(r, r"paris"),
    "where-is-london": lambda r: map_of(r, "london") and said(r, r"london"),
    "directions": lambda r: (shown(r, r"openstreetmap|google\.com/maps|directions") or said(r, CANT))
                            and not said(r, r"here's singapore\.$"),
    "map-other-city": lambda r: (map_of(r, "berlin") and said(r, r"berlin")) or
                                (said(r, r"berlin") and said(r, CANT + r"|link")),
    "timer-5": lambda r: shown(r, r"timer|5:00|05:00|countdown") and said(r, r"\b5\b|five"),
    "timer-terse": lambda r: shown(r, r"timer|20:00|countdown") and said(r, r"\b20\b|twenty"),
    "eggs": lambda r: shown(r, r"timer|countdown") and said(r, r"\b([6-9]|1[0-2])\b|six|seven|eight|nine|ten|eleven|twelve"),
    "weather": lambda r: said(r, r"°|degrees") and not said(r, r"made[- ]up") or said(r, CANT),
    "weather-city": lambda r: (((said(r, r"new york") or did(r, r"weather for new york")) and said(r, r"°|degrees")) or
                               (said(r, CANT) and not said(r, r"singapore"))) and not said(r, r"made[- ]up forecast"),
    "umbrella": lambda r: said(r, r"umbrella|rain|dry|showers") and (said(r, r"°|degrees|%|forecast") or said(r, CANT))
                          and not said(r, r"\btomorrow\b"),
    "messages": lambda r: said(r, r"mum|jane|alex") and not said(r, r"only know a few things"),
    "jane": lambda r: said(r, r"jane") and said(r, r"12:30|lunch"),
    "email": lambda r: said(r, r"jane|email|dbs|inbox") and (r.get("open") or said(r, r"email")),
    "reply-mum": lambda r: said(r, r"can't|cannot|doesn't|isn't able|not able|only reads|can only read") and said(r, r"send|reply"),
    "packing": lambda r: shown(r, r"packing") and said(r, r"packing"),
    "find-photo": lambda r: shown(r, r"beach"),
    "search-files": lambda r: said(r, r"budget 2026") or shown(r, r"budget 2026"),
    "louder": lambda r: said(r, r"volume|louder") and did(r, r"volume"),
    "bigger-text": lambda r: said(r, r"bigger|larger"),
    "close-it": lambda r: said(r, r"closed|nothing'?s open|nothing is open"),
    "wifi": lambda r: said(r, r"wi-?fi|internet|online|connected"),
    "open-website": lambda r: src(r, r"youtube") or (said(r, r"youtube") and (said(r, r"link|new tab") or shown(r, r"youtube\.com"))),
    "open-google": lambda r: src(r, r"google") or (said(r, r"google") and (said(r, r"link|new tab|search") or shown(r, r"google\.com"))),
    "news": lambda r: said(r, CANT) or shown(r, r"news") or (said(r, r"news") and said(r, r"wikipedia|current events|according")),
    "recipe": lambda r: shown(r, r"chicken rice|ingredients") or (said(r, r"ingredients") and said(r, r"chicken")),
    "calc": lambda r: said(r, r"12\.68|12\.675"),
    "capital": lambda r: said(r, r"canberra"),
    # An AI's longer answers go on the screen (show_page), with a sentence in the reply: either place counts.
    "explain-quantum": lambda r: (said(r, r"qubit|quantum") and len(r.get("answer") or "") > 120 or shown(r, r"qubit|superposition"))
                                 and not said(r, r"only know a few things"),
    "write-email": lambda r: (written(r, r"friday|boss|dear|hi ") or said(r, r"(dear|hi) .{0,20},|friday off")) and not said(r, r"\bi'm chat computer\b"),
    "poem": lambda r: (shown(r, r"\b(sea|waves?|tides?|shore)\b") or said(r, r"\b(sea|waves?|tides?|shore)\b"))
                      and not shown(r, r"packing") and not said(r, NO_AI),
    "translate": lambda r: said(r, r"arigat") or shown(r, r"arigat"),
    "trip-plan": lambda r: shown(r, r"day 1|day one|ubud|bali") or (said(r, r"day 1|ubud") and said(r, r"bali")),
    "advice": lambda r: (said(r, r"\bbed(time)?\b|caffeine|routine|wind down|screens? (before|an hour|off)")
                         or shown(r, r"\bbed(time)?\b|caffeine|routine")) and not said(r, r"only know a few things"),
    "code": lambda r: (said(r, r"def |\[::-1\]") or shown(r, r"def |\[::-1\]")) and not shown(r, r"packing"),
    "date-today": lambda r: said(r, r"monday|tuesday|wednesday|thursday|friday|saturday|sunday|\b20\d\d\b"),
    "time-now": lambda r: said(r, r"\b\d{1,2}[:.]\d\d\b|o'clock"),
    "joke": lambda r: len(r.get("answer") or "") > 20 and not said(r, r"only know a few things|demo"),
    "how-install": lambda r: said(r, r"coming soon|not (yet )?(available|out|ready)|isn't (out|available|ready)") and
                             said(r, r"windows|mac"),
    "download": lambda r: said(r, r"coming soon|not (yet )?(available|out|ready)|isn't (out|available|ready)"),
    "price": lambda r: said(r, r"\bfree\b") and not said(r, r"subscription|\bpro or max\b|claude plan|not free|isn't free"),
    "mac": lambda r: said(r, r"\byes\b|works on") and said(r, r"m1|m2|apple silicon"),
    "windows-safe": lambda r: said(r, r"\bno\b|stays|keeps|next to") and said(r, r"windows"),
    "requirements": lambda r: said(r, r"uefi|70 ?gb|m1|m2") and not said(r, r"subscription|\bpro or max\b|claude plan"),
    "old-laptop": lambda r: said(r, r"next to windows|installs|chat computer") and (said(r, r"windows 10") or said(r, r"uefi|70 ?gb")),
    "privacy": lambda r: said(r, r"browser|this page|on your (computer|device)|stays") and said(r, r"private|stays|never|doesn't leave|don't leave"),
    "is-it-free": lambda r: said(r, r"\bfree\b") and not said(r, r"subscription|\bpro or max\b|claude plan|not free|isn't free"),
    "airpods": lambda r: said(r, r"\bno\b|optional|don't need|do not need|not needed"),
    "are-you-claude": lambda r: said(r, r"written in advance|scripted|qwen|own model|this computer's model"),
    "who-made": lambda r: said(r, r"independent") and said(r, r"qwen|own engine|open model"),
    "anthropic": lambda r: said(r, r"\bno\b|independent") and not said(r, r"\byes\b"),
    "cant-do": lambda r: said(r, r"can't|cannot|only|not") and said(r, r"this page|here|demo|written in advance|in this browser"),
    "control-my-pc": lambda r: said(r, r"can't|cannot|no\b|not from") and said(r, r"made up|this page|website|browser"),
    "real-or-fake": lambda r: said(r, r"made up|not real|aren't real|pretend|examples|made-up"),
    "gibberish": lambda r: said(r, r"try|didn't (catch|get)|not sure"),
    "emoji": lambda r: said(r, r"\b(hi|hello|hey)\b"),
    "question-mark": lambda r: said(r, r"try|can|" + GET_ONE),
    "chinese": lambda r: re.search(r"[一-鿿]", r.get("answer") or "") is not None,
    "spanish": lambda r: said(r, r"\b(computadora|ordenador|chat computer)\b"),
    "malay": lambda r: said(r, r"\b(komputer|chat computer)\b") and not said(r, r"only know a few things|cuaca"),
    "rude": lambda r: said(r, r"sorry|try|can") and not said(r, r"only know a few things"),
    "long": lambda r: said(r, r"father|dad|\b(he|him|his)\b") and said(r, r"talk|no menus|no desktop|simple")
                      and said(r, r"set ?up|install"),
    # Needing the internet: answered from what was looked up, never "I can't go on the internet".
    "haze-trace": lambda r: online(r) and len(answers(r)) == 5 and turn(r, 0, r"written in advance|qwen|model")
                            and not turn(r, 0, r"cannot|can't|don't") and turn(r, 2, r"haze|smoke|fires?\b|burning|air quality|\baqi\b|pm2\.?5|pollution"),
    "haze-now": lambda r: online(r) and said(r, r"air quality|\baqi\b|pm2\.?5|\b(good|moderate|unhealthy|hazardous)\b|\bhaze\b"),
    "pm": lambda r: online(r) and said(r, r"lawrence wong"),
    "world-cup": lambda r: online(r) and (said(r, r"\bspain\b") or shown(r, r"\bspain\b")),
    "look-it-up": lambda r: online(r) and (said(r, r"safdie") or shown(r, r"safdie")),
    "search-population": lambda r: online(r) and (said(r, r"\b[56]([.,]\d+)? million|\b[56],\d{3},\d{3}\b")
                                                   or shown(r, r"\b[56]([.,]\d+)? million")),
    "headlines": lambda r: online(r) and (shown(r, r"in the news") or did(r, r"news")),
    "news-yesterday": lambda r: online(r) and (shown(r, r"in the news") or did(r, r"news")),
    "why-no-internet": lambda r: online(r) and said(r, r"\bi can\b|look (things|it) up|wikipedia|\bsearch"),
    "tell-me-more": lambda r: online(r) and turn(r, 0, r"haze|smoke|fires?\b|pollution") and turn(r, 1, r"1997|2015|2019|2006")
                              and not turn(r, 0, r"air quality index"),
    # Which AI answers, and choosing it (the spark beside +: your own AI, its model and effort; / for this browser's).
    "which-model-now": lambda r: said(r, r"written in advance|qwen|own model|in this browser") and not catch_all(r),
    "use-chatgpt": lambda r: said(r, r"openai|api key") and said(r, r"spark|openrouter") and not catch_all(r),
    "use-claude": lambda r: said(r, r"anthropic|api key") and said(r, r"spark|openrouter") and not catch_all(r),
    "change-model": lambda r: said(r, r"spark") and said(r, r"\bmodels?\b") and not catch_all(r),
    "think-harder": lambda r: said(r, r"effort|how hard|think") and not catch_all(r)
                              and not said(r, r"\bI('ll| will) (think|try) (harder|more)"),
    "smarter-ai": lambda r: said(r, r"spark|claude|chatgpt|gemini|openrouter") and not catch_all(r),
    "run-in-browser": lambda r: said(r, r"run my model in this browser|in (this|your) browser") and not catch_all(r),
    "connect-gmail": lambda r: said(r, r"gmail") and said(r, r"\bapp\b|\+ in messages|can't connect") and not shown(r, r"jane lim"),
    "call-me": lambda r: said(r, r"\bren\b") and did(r, r"name"),
    "i-live-tokyo": lambda r: said(r, r"tokyo") and (did(r, r"place") or said(r, r"got it|noted|kept|i'll remember")),
    "weather-follow": lambda r: turn(r, 0, r"paris") and turn(r, 1, r"tomorrow") and turn(r, 1, r"paris") and turn(r, 1, r"°|degrees"),
    "timer-words": lambda r: shown(r, r"15:00") and said(r, r"\b15\b|fifteen"),
    "timer-cancel": lambda r: turn(r, 0, r"\b3\b|three") and turn(r, 1, r"closed|cancel|gone|stopped|removed") and not r.get("open"),
}

# --- The failures of 4 November 2026, as checks over every case.
CATCH_ALL = (r"\bI can't (answer that|[a-z ]{3,30}) yet\b|"
             r"\bTo (answer that|write an? [a-z]+|write code|translate|explain that|plan that|give advice), I need (a model|an AI)\b")
# Only a model can answer these: without one, saying so (and how to get one) is the page's best.
NEEDS_AI = {"recipe", "write-email", "poem", "translate", "trip-plan", "advice", "code", "tell-me-more", "haze-trace"}


def catch_all(r: R) -> bool:
    return any(re.search(CATCH_ALL, a, re.I) for a in answers(r))


def rounds(r: R) -> list[dict]:
    return r.get("rounds") or []


def tool_calls(r: R) -> list[tuple[str, str]]:
    out = []
    for x in rounds(r):
        for c in ((x.get("out") or {}).get("tool_calls") or []):
            f = c.get("function") or {}
            out.append((f.get("name") or "", f.get("arguments") or ""))
    return out


def tool_results(r: R) -> list[str]:
    return [m.get("content") or "" for m in (r.get("messages") or []) if m.get("role") == "tool"]


# The words that ask for each tool (any of the conversation's turns).
TOOL_WORDS = {
    "weather": r"weather|rain|umbrella|forecast|temperature|hot|cold|sunny|degrees|jacket|outside",
    "show_map": r"\bmaps?\b|where is|where's|directions|get (from|to)|how far|route",
    "make_timer": r"timer|countdown|minutes?|\bmin\b|remind|tell me when|eggs?|alarm",
    "youtube_search": r"youtube|videos?|music|songs?|play|watch|lofi|jazz|piano",
    "close": r"close|stop|cancel|hide|turn (it )?off|get rid",
}


def general(r: R) -> dict[str, bool]:
    """Each general check: True where it passes (the failure isn't there)."""
    asked = " ".join(r.get("turns") or [r.get("query") or ""])
    calls = tool_calls(r)
    names = [n for n, _ in calls]
    said_all = " ".join(answers(r))
    stray = [n for n in names if n in TOOL_WORDS and not re.search(TOOL_WORDS[n], asked, re.I)]
    singapore_unasked = not re.search(r"singapore|weather|rain|umbrella|haze|air|pm|prime minister|population|marina|orchard|"
                                      r"news|headlines|happen|time|day|date", asked, re.I)
    return {
        "catch_all": not catch_all(r) or (r.get("model") != "web" and r["id"] in NEEDS_AI),
        "stale_step": not re.search(r"use this computer's model", said_all, re.I) or
                      re.search(r"run my model in this browser|spark", said_all, re.I) is not None,
        "empty": all(re.search(r"\S", a) for a in answers(r)) and bool(answers(r)),
        "round_cap": len(rounds(r)) < 8 and len(set(calls)) == len(calls),
        "bad_input": not any(re.search(r"^That (box|isn't)|isn't a|There's no tool called", t) for t in tool_results(r)),
        "stray_tool": not stray,
        "time_stamp": not (singapore_unasked and (re.search(r"singapore", said_all, re.I) or
                                                  any(re.search(r"singapore", a, re.I) for _, a in calls))),
    }


def evaluate(results: list[R]) -> dict[str, dict[str, Any]]:
    out = {}
    for r in results:
        cid = r["id"]
        ok = bool(DONE[cid](r)) and (cid in DEAD_END_OK or not dead_end(r)) if cid in DONE else None
        out[cid] = {"pass": ok, "errors": bool(r.get("errors")), "general": general(r) if "answers" in r else {},
                    "first_ms": r.get("first_ms"), "total_ms": r.get("total_ms")}
    return out


def agreement(pred: dict[str, bool], labels: dict[str, Any]) -> str:
    """How a check's verdicts agree with the hand labels' pass/fail: TPR, the share of labelled failures it fails; TNR,
    the share of labelled passes it passes."""
    both = [k for k in pred if k in labels and labels[k].get("pass") is not None]
    fails = [k for k in both if not labels[k]["pass"]]
    passes = [k for k in both if labels[k]["pass"]]
    tp = sum(1 for k in fails if not pred[k])
    tn = sum(1 for k in passes if pred[k])
    return (f"TPR {tp}/{len(fails)} ({tp / max(1, len(fails)):.0%}), TNR {tn}/{len(passes)} ({tn / max(1, len(passes)):.0%})")


def report(res: dict[str, dict[str, Any]], labels: dict[str, Any] | None = None) -> str:
    cases = {c["id"]: c for c in json.loads((HERE / "cases.json").read_text())["cases"]}
    scored = [k for k, v in res.items() if v["pass"] is not None]
    ok = sum(res[k]["pass"] for k in scored)
    lines = [f"pass: {ok}/{len(scored)} ({ok / max(1, len(scored)):.0%})"]
    by: dict[str, list[int]] = {}
    for k in scored:
        a = cases.get(k, {}).get("intent", "?")
        by.setdefault(a, [0, 0])
        by[a][0] += res[k]["pass"]
        by[a][1] += 1
    lines.append("by intent: " + ", ".join(f"{a} {x}/{n}" for a, (x, n) in by.items()))
    lines.append("not passed: " + ", ".join(k for k in scored if not res[k]["pass"]))
    errs = [k for k, v in res.items() if v["errors"]]
    lines.append(f"page errors: {len(errs)}" + (f" ({', '.join(errs)})" if errs else ""))
    firsts = sorted(v["first_ms"] for v in res.values() if v.get("first_ms") is not None)
    if firsts:
        lines.append(f"first words after: median {firsts[len(firsts) // 2] / 1000:.1f} s, longest {firsts[-1] / 1000:.1f} s")
    gen = [k for k, v in res.items() if v.get("general")]
    if gen:
        names = list(res[gen[0]]["general"])
        lines.append("general checks (cases failing each): " + ", ".join(
            f"{n} {sum(1 for k in gen if not res[k]['general'][n])}" for n in names))
        for n in names:
            bad = [k for k in gen if not res[k]["general"][n]]
            if bad:
                lines.append(f"  {n}: {', '.join(bad)}")
    if labels:
        both = [k for k in scored if k in labels and labels[k].get("pass") is not None]
        dis = [k for k in both if labels[k]["pass"] != res[k]["pass"]]
        lines.append(f"checks vs hand labels: {len(both) - len(dis)}/{len(both)} agree" + (f"; disagree: {dis}" if dis else ""))
        lines.append("  the case checks: " + agreement({k: res[k]["pass"] for k in scored}, labels))
        if gen:
            every = {k: res[k]["pass"] is not False and all(res[k]["general"].values()) for k in gen}
            lines.append("  the case checks and the general ones together: " + agreement(every, labels))
    return "\n".join(lines)


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("results")
    ap.add_argument("--labels", default="")
    a = ap.parse_args()
    results = json.loads(Path(a.results).read_text())
    labels = json.loads(Path(a.labels).read_text()).get("labels") if a.labels else None
    print(report(evaluate(results), labels))


if __name__ == "__main__":
    main()
