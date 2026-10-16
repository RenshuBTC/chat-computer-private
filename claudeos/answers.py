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
        # "yes but only the PDFs" is a different instruction, not a plain yes.
        if any(w in ("but", "except", "only", "instead", "not") for w in words[1:]):
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


def is_sleep(text: str) -> bool:
    t = normalize(text)
    return t in _SLEEP or t.rstrip(".") in _SLEEP


def strip_wake_word(text: str, wake_words: list[str]) -> tuple[bool, str]:
    """If the utterance starts with a wake word ("Claude, open YouTube"), return (True, rest)."""
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
            # "the cloud is..." must not wake ClaudeOS; only a leading wake phrase counts.
            return True, raw[m.end():].strip()
    return False, raw
