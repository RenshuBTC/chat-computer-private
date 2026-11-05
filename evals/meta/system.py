"""What Chat Computer's own kernel gives Meta's model, read from the kernel's own source, so that the evals test the
system as it is and never a copy of it: the system prompt (os/kernel/src/assistant.rs: SYSTEM, with META_WHO), the
tools (os/kernel/src/tools.json), the request's form (meta_body: max_completion_tokens META_TOKENS), the rounds of
tools one message may use (TOOL_ROUNDS) and what the last of them says (last_round, NOT_RUN), and how a message from
the person starts (clock::short_text: "[Thu 5 Nov 2026 10:12] ...")."""
from __future__ import annotations

import json
import re
import subprocess
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
ASSISTANT = ROOT / "os" / "kernel" / "src" / "assistant.rs"
TOOLS_JSON = ROOT / "os" / "kernel" / "src" / "tools.json"


def rust_str(source: str, name: str) -> str:
    """A `const NAME: &str = "...";` of the Rust source, as the string it is (escapes and line continuations)."""
    m = re.search(r"const " + re.escape(name) + r': &str = "', source)
    if not m:
        raise KeyError(name)
    i, out = m.end(), []
    while True:
        c = source[i]
        if c == '"':
            return "".join(out)
        if c != "\\":
            out.append(c)
            i += 1
            continue
        n = source[i + 1]
        if n == "\n":                       # a line continuation: the newline and the next line's indent go
            i += 2
            while source[i] in " \t\n":
                i += 1
        elif n == "u":                      # \u{201C}
            j = source.index("}", i)
            out.append(chr(int(source[i + 3:j], 16)))
            i = j + 1
        else:
            out.append({"n": "\n", "t": "\t", "\\": "\\", '"': '"', "'": "'", "0": "\0"}[n])
            i += 2


def rust_int(source: str, name: str) -> int:
    return int(re.search(r"const " + re.escape(name) + r": (?:u32|usize|u64) = (\d+);", source).group(1))


def source(path: Path, rev: str | None = None) -> str:
    """A file of the repository as it is, or as it was at a git revision (to evaluate an earlier system)."""
    if not rev:
        return path.read_text()
    return subprocess.run(["git", "show", f"{rev}:{path.relative_to(ROOT).as_posix()}"], cwd=ROOT, capture_output=True,
                          text=True, check=True).stdout


def not_yet(rev: str | None = None) -> str:
    """system.rs status()'s last line: what Chat Computer's own system can't do yet (as computer_status says it)."""
    src = source(ROOT / "os" / "kernel" / "src" / "system.rs", rev)
    m = re.search(r'String::from\(\s*"(Not yet in Chat Computer\'s own system:.*?)",?\s*\)', src, re.S)
    return re.sub(r"\\\n\s*", "", m.group(1))


def kernel(rev: str | None = None) -> dict:
    src = source(ASSISTANT, rev)
    rounds = rust_int(src, "TOOL_ROUNDS")
    last = re.search(r'fn last_round\(\) -> String \{\s*format!\(\s*"(.*?)"\s*\)', src, re.S).group(1)
    last = re.sub(r"\\\n\s*", "", last).replace("{TOOL_ROUNDS}", str(rounds))
    return {"system": rust_str(src, "SYSTEM"), "meta_who": rust_str(src, "META_WHO"), "model": rust_str(src, "META_MODEL"),
            "max_tokens": rust_int(src, "META_TOKENS"), "rounds": rounds, "last_round": last,
            "not_run": rust_str(src, "NOT_RUN"), "tools": json.loads(source(TOOLS_JSON, rev)), "rev": rev or "working tree",
            # Whether the kernel shows words written with tool calls that will run (it stopped doing so for Meta's
            # model when evals/meta found them on the screen in two answers of three).
            "hides_narration": "MOST_DOING" in src,
            # Whether the kernel asks again, once, when an answer says the PC turns off or restarts with no tool
            # call (claims_power): its words, as the kernel has them.
            "reask_power": power_words(src)}


def power_words(src: str):
    m = re.search(r"fn claims_power\(text: &str\) -> bool \{(.*?)\n\}", src, re.S)
    if not m:
        return None
    lists = re.findall(r"\[([^\]]*)\]", m.group(1))
    def words(body: str) -> list[str]:
        return [re.sub(r"\\u\{([0-9a-f]+)\}", lambda x: chr(int(x.group(1), 16)), w) for w in re.findall(r'"((?:[^"\\]|\\.)*)"', body)]
    return {"says": words(lists[0]), "not": words(lists[1])}


def claims_power(k: dict, text: str) -> bool:
    w = k.get("reask_power")
    if not w:
        return False
    t = text.lower()
    return any(x in t for x in w["says"]) and not any(x in t for x in w["not"])


def system_prompt(k: dict, model: str, notes: list[str] | None = None) -> str:
    """As the kernel's system_prompt makes it for Meta's model (which isn't given pictures: no {sees})."""
    s = k["system"].replace("{who}", k["meta_who"]).replace("{model}", model).replace("{sees}", "")
    if notes:
        s += "\n\nYour notes about the person, from earlier conversations (made with remember):"
        for n in notes:
            s += "\n• " + n
    return s


DAYS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"]


def stamp(t: time.struct_time) -> str:
    """clock::short_text: [Thu 5 Nov 2026 10:12]."""
    return f"[{DAYS[t.tm_wday]} {t.tm_mday} {MONTHS[t.tm_mon - 1]} {t.tm_year} {t.tm_hour:02}:{t.tm_min:02}]"


if __name__ == "__main__":
    k = kernel()
    print(system_prompt(k, k["model"]))
    print("---", len(k["tools"]), "tools;", k["rounds"], "rounds;", k["max_tokens"], "tokens;", k["last_round"])
