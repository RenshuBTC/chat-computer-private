"""Pass/fail checks for the website's answers (evals/web/run.js results), one per case, written from the error
analysis of the scripted baseline (labels/scripted-baseline.json): does the answer (and what opened between the
sidebars) do what a first-time visitor asked, or say honestly what this page can't do?

    python -m evals.web.evaluate RESULTS.json [--labels evals/web/labels/scripted-baseline.json]

With --labels it says where the checks and the hand labels disagree (how far to trust the checks)."""
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
NOT_STUCK = r"try|use this computer's model|you can|say|type"
NO_AI = r"without an ai|written in advance"     # the page's honest note when only an AI could answer


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


def music(r: R) -> bool:
    return (did(r, r"play") or shown(r, r"piano|music|youtube")) and not said(r, r"\bcan't\b.*\bplay\b")


DONE: dict[str, Callable[[R], bool]] = {
    "what-is-this": lambda r: said(r, r"chat computer") and said(r, r"talk|type|touch") and said(r, r"try"),
    "how-work": lambda r: said(r, r"talk|type|touch") and said(r, r"try|do it|does it"),
    "what-can-do": lambda r: said(r, r"music|maps?|messages|files|websites") and said(r, r"try"),
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
    "map-tokyo": lambda r: shown(r, r"openstreetmap") and said(r, r"tokyo"),
    "map-short": lambda r: shown(r, r"openstreetmap") and said(r, r"paris"),
    "where-is-london": lambda r: shown(r, r"openstreetmap") and said(r, r"london"),
    "directions": lambda r: (shown(r, r"openstreetmap|google\.com/maps|directions") or said(r, CANT))
                            and not said(r, r"here's singapore\.$"),
    "map-other-city": lambda r: (shown(r, r"openstreetmap") and said(r, r"berlin")) or
                                (said(r, r"berlin") and said(r, CANT + r"|link")),
    "timer-5": lambda r: shown(r, r"timer|5:00|05:00|countdown") and said(r, r"\b5\b|five"),
    "timer-terse": lambda r: shown(r, r"timer|20:00|countdown") and said(r, r"\b20\b|twenty"),
    "eggs": lambda r: shown(r, r"timer|countdown") and said(r, r"\b([6-9]|1[0-2])\b|six|seven|eight|nine|ten|eleven|twelve"),
    "weather": lambda r: said(r, r"°|degrees") and not said(r, r"made[- ]up") or said(r, CANT),
    "weather-city": lambda r: ((said(r, r"new york") and said(r, r"°|degrees")) or (said(r, CANT) and not said(r, r"singapore")))
                              and not said(r, r"made[- ]up forecast"),
    "umbrella": lambda r: said(r, r"umbrella|rain|dry|showers") and (said(r, r"°|degrees|%|forecast") or said(r, CANT)),
    "messages": lambda r: said(r, r"mum|jane|alex") and not said(r, r"only know a few things"),
    "jane": lambda r: said(r, r"jane") and said(r, r"12:30|lunch"),
    "email": lambda r: said(r, r"jane|email|dbs|inbox") and (r.get("open") or said(r, r"email")),
    "reply-mum": lambda r: said(r, r"can't|cannot|doesn't|isn't able|not able|only reads|can only read") and said(r, r"send|reply"),
    "packing": lambda r: shown(r, r"packing") and said(r, r"packing"),
    "find-photo": lambda r: shown(r, r"beach"),
    "search-files": lambda r: said(r, r"budget") or shown(r, r"budget"),
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
    "write-email": lambda r: (shown(r, r"friday|boss|dear|hi ") or said(r, r"(dear|hi) .{0,20},|friday off")) and not said(r, r"\bi'm chat computer\b"),
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
    "price": lambda r: said(r, r"\bfree\b") and not said(r, r"subscription|\bpro or max\b|claude plan"),
    "mac": lambda r: said(r, r"\byes\b|works on") and said(r, r"m1|m2|apple silicon"),
    "windows-safe": lambda r: said(r, r"\bno\b|stays|keeps|next to") and said(r, r"windows"),
    "requirements": lambda r: said(r, r"uefi|70 ?gb|m1|m2") and not said(r, r"subscription|\bpro or max\b|claude plan"),
    "old-laptop": lambda r: said(r, r"windows 10") or (said(r, r"\byes\b") and said(r, r"uefi|70 ?gb")),
    "privacy": lambda r: said(r, r"browser|this page|on your (computer|device)|stays") and said(r, r"private|stays|never|doesn't leave|don't leave"),
    "is-it-free": lambda r: said(r, r"\bfree\b") and not said(r, r"subscription|\bpro or max\b|claude plan"),
    "airpods": lambda r: said(r, r"\bno\b|optional|don't need|not needed") and said(r, r"type|touch"),
    "are-you-claude": lambda r: said(r, r"written in advance|scripted|qwen|own model|this computer's model"),
    "who-made": lambda r: said(r, r"independent") and said(r, r"qwen|own engine|open model"),
    "anthropic": lambda r: said(r, r"\bno\b|independent") and said(r, r"qwen|open model|own engine"),
    "cant-do": lambda r: said(r, r"can't|cannot|only|not") and said(r, r"this page|here|demo"),
    "control-my-pc": lambda r: said(r, r"can't|cannot|no\b|not from") and said(r, r"made up|this page|website|browser"),
    "real-or-fake": lambda r: said(r, r"made up|not real|aren't real|pretend|examples|made-up"),
    "gibberish": lambda r: said(r, r"try|didn't (catch|get)|not sure"),
    "emoji": lambda r: said(r, r"\b(hi|hello|hey)\b"),
    "question-mark": lambda r: said(r, r"try|can"),
    "chinese": lambda r: re.search(r"[一-鿿]", r.get("answer") or "") is not None,
    "spanish": lambda r: said(r, r"\b(hola|es|puedes|computadora|ordenador)\b"),
    "malay": lambda r: said(r, r"\b(ini|anda|boleh|komputer)\b") or (said(r, r"chat computer") and not said(r, r"only know a few things")),
    "rude": lambda r: said(r, r"sorry|try|can") and not said(r, r"only know a few things"),
    "long": lambda r: said(r, r"father|dad|\b(he|him|his)\b") and said(r, r"talk|no menus|no desktop|simple")
                      and said(r, r"set ?up|install"),
    # Needing the internet: answered from what was looked up, never "I can't go on the internet".
    "haze-trace": lambda r: online(r) and len(answers(r)) == 5
                            and turn(r, 2, r"haze|smoke|fires?\b|burning|air quality|\baqi\b|pm2\.?5|pollution"),
    "haze-now": lambda r: online(r) and said(r, r"air quality|\baqi\b|pm2\.?5|\b(good|moderate|unhealthy|hazardous)\b|\bhaze\b"),
    "pm": lambda r: online(r) and said(r, r"lawrence wong"),
    "world-cup": lambda r: online(r) and (said(r, r"\bspain\b") or shown(r, r"\bspain\b")),
    "look-it-up": lambda r: online(r) and (said(r, r"safdie") or shown(r, r"safdie")),
    "search-population": lambda r: online(r) and (said(r, r"\b[56]([.,]\d+)? million|\b[56],\d{3},\d{3}\b")
                                                   or shown(r, r"\b[56]([.,]\d+)? million")),
    "headlines": lambda r: online(r) and (shown(r, r"in the news") or did(r, r"news")),
    "news-yesterday": lambda r: online(r) and (shown(r, r"in the news") or did(r, r"news")),
    "why-no-internet": lambda r: online(r) and said(r, r"\bi can\b|look (things|it) up|wikipedia|\bsearch"),
    "tell-me-more": lambda r: online(r) and turn(r, 0, r"haze|smoke|fires?\b|pollution") and turn(r, 1, r"1997|2015|2019|2006"),
}


def evaluate(results: list[R]) -> dict[str, dict[str, Any]]:
    out = {}
    for r in results:
        cid = r["id"]
        ok = bool(DONE[cid](r)) and (cid in DEAD_END_OK or not dead_end(r)) if cid in DONE else None
        out[cid] = {"pass": ok, "errors": bool(r.get("errors")),
                    "first_ms": r.get("first_ms"), "total_ms": r.get("total_ms")}
    return out


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
    if labels:
        both = [k for k in scored if k in labels and labels[k].get("pass") is not None]
        dis = [k for k in both if labels[k]["pass"] != res[k]["pass"]]
        lines.append(f"checks vs hand labels: {len(both) - len(dis)}/{len(both)} agree" + (f"; disagree: {dis}" if dis else ""))
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
