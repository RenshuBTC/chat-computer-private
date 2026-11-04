"""Understanding short spoken answers: yes / no / something else, and "stop" commands."""
from __future__ import annotations

import re

_YES = {
    "yes", "yeah", "yep", "yup", "ya", "yah", "sure", "ok", "okay", "alright", "all right", "go", "go ahead",
    "do it", "please do", "confirm", "confirmed", "correct", "right", "absolutely", "definitely", "of course",
    "affirmative", "proceed", "continue", "carry on", "fine", "sounds good", "go for it", "yes please",
    "please", "sure thing", "why not", "that's fine", "that's right", "uh huh", "mm hmm", "can", "ok la",
    "okay la", "can can", "yes la", "do that", "send it", "delete it", "install it", "go on",
}
_NO = {
    "no", "nope", "nah", "don't", "do not", "dont", "stop", "cancel", "never mind", "nevermind", "wait",
    "hold on", "hang on", "not now", "no thanks", "no thank you", "negative", "abort", "forget it",
    "don't do it", "dont do it", "not yet", "no no", "cannot", "can't", "no la", "skip", "skip it", "leave it",
}
_FILLER = re.compile(r"\b(um+|uh+|er+|ah+|hmm+|well|so|like|just|claude)\b")
# After a yes-word: words that take it back ("okay, wait", "yeah no", "ok stop", "yes, never mind")...
_TAKE_BACK = {"no", "nope", "nah", "don't", "dont", "stop", "wait", "cancel", "hold", "hang", "abort", "cannot",
              "can't", "cant", "never"}
# ...or make it a different instruction or a question ("yes but only the PDFs", "sure, what does it cost?").
_NOT_PLAIN = {"but", "except", "only", "instead", "not", "what", "why", "how", "which", "who", "where", "when"}


def normalize(text: str) -> str:
    t = text.lower().strip()
    t = re.sub(r"[^\w\s']", " ", t)
    t = _FILLER.sub(" ", t)
    return re.sub(r"\s+", " ", t).strip()


def classify_answer(text: str) -> str:
    """'yes', 'no' or 'unclear' for a reply to a yes/no question."""
    t = normalize(text)
    if not t:
        return "unclear"
    if t in _YES:
        return "yes"
    if t in _NO:
        return "no"
    words = t.split()
    # Short replies that start with a clear word: "yes do it", "no not that one".
    first2 = " ".join(words[:2])
    if words[0] in ("no", "nope", "nah", "don't", "dont", "cancel", "stop", "wait") or first2 in _NO:
        return "no"
    if (words[0] in ("yes", "yeah", "yep", "yup", "sure", "ok", "okay", "alright") or first2 in _YES) and len(words) <= 6:
        # Only a plain yes is a yes: "okay, wait" isn't, and nor is "yes but only the PDFs".
        if any(w in _TAKE_BACK for w in words[1:]):
            return "no"
        if any(w in _NOT_PLAIN for w in words[1:]):
            return "unclear"
        return "yes"
    return "unclear"


_STOP = {
    "stop", "stop it", "stop talking", "be quiet", "quiet", "shush", "shh", "shut up", "cancel", "cancel that",
    "never mind", "nevermind", "forget it", "that's enough", "enough", "okay stop", "ok stop", "stop stop",
    "hold on", "wait", "wait wait", "hang on", "claude stop", "abort",
}
_SLEEP = {
    "go to sleep", "stop listening", "sleep", "goodnight", "good night", "goodbye", "bye", "that's all",
    "that's all for now", "thanks that's all", "thank you that's all", "you can sleep", "go sleep",
}


def is_stop(text: str) -> bool:
    t = normalize(text.replace("claude", ""))
    return t in _STOP


_RINGING_DONE = _STOP | {
    "ok", "okay", "ok thanks", "okay thanks", "thanks", "thank you", "got it", "alright", "all right", "i'm up",
    "im up", "i'm awake", "im awake", "dismiss", "off", "turn it off", "switch it off", "stop the alarm",
    "stop the timer", "turn off the alarm", "turn the alarm off", "alarm off", "timer off", "silence", "yes", "yeah",
}


def stops_ringing_only(text: str) -> bool:
    """What someone says to a timer or alarm that's ringing, meaning only "that's enough" ("stop", "okay thanks",
    "I'm up"): it stops it, and isn't a request for the model."""
    t = normalize(text.replace("claude", "")).rstrip(".!")
    return t in _RINGING_DONE


def is_snooze(text: str) -> bool:
    return "snooze" in normalize(text)


# A web address typed or said alone, as into a browser's address bar: "youtube.com", "open bbc.co.uk", a whole link.
# Its last part is a known ending: the common ones, or any two letters (a country's), except those that are more
# often a file's (notes.md, main.rs, run.sh), which the model finds on the computer instead. (docs/demo/host.js has the
# same, for the website.)
_TLD = ("com|org|net|edu|gov|mil|int|info|biz|io|ai|app|dev|co|me|tv|xyz|site|online|store|shop|tech|blog|news|"
        "cloud|page|live|art|design|studio|agency|world|today|space|website|fun|club|life|email|link|pro|mobi|name|"
        "museum|travel|jobs|asia|wiki|games|game|music|video|social|network|digital|media|global|group|one|top|"
        "health|bank|money|finance|law|school|academy|university|college|press|photo|photos|gallery|"
        r"(?!(?:md|py|sh|rs|cc|so|ps)\b)[a-z]{2}")
_ADDRESS = re.compile(
    r"^\s*(?:(?:please\s+)?(?:open|go to|goto|visit|load|launch|browse to|take me to)\s+)?"
    r"((?:https?://)?(?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+(?:" + _TLD + r")(?::\d{2,5})?(?:[/?#]\S*)?)"
    r"(?:,?\s+(?:please|pls|thanks|thank you))?\s*[.!]*\s*$", re.IGNORECASE)


def typed_address(text: str) -> str | None:
    """The site a web address typed or said alone names, as an https:// address ("youtube.com" ->
    "https://youtube.com"); None for anything else."""
    m = _ADDRESS.match(text or "")
    if not m:
        return None
    raw = m.group(1).rstrip(".,!")
    return raw if re.match(r"https?://", raw, re.IGNORECASE) else "https://" + raw


def is_sleep(text: str) -> bool:
    t = normalize(text)
    return t in _SLEEP or t.rstrip(".") in _SLEEP


def strip_wake_word(text: str, wake_words: list[str]) -> tuple[bool, str]:
    """If the utterance starts with a wake word ("Computer, open YouTube"), return (True, rest)."""
    raw = text.strip()
    low = raw.lower()
    variants: list[str] = []
    for w in wake_words:
        w = w.lower().strip()
        variants += [w, w.replace("claude", "cloud"), w.replace("claude", "clawed"), w.replace("claude", "claud"),
                     w.replace("claude", "clod")]
    variants = sorted(set(variants), key=len, reverse=True)
    for v in variants:
        m = re.match(r"^\W*" + re.escape(v) + r"\b[\s,.!?:;-]*", low)
        if m:
            # "the cloud is..." must not wake Chat Computer; only a leading wake phrase counts.
            return True, raw[m.end():].strip()
    return False, raw
