"""Long-term memory: short facts about you that Chat Computer keeps between conversations
(~/ChatComputer/memory/memory.md). You can read and edit the file yourself."""
from __future__ import annotations

import re
import time

from .config import Paths, atomic_write

HEADER = "# What Chat Computer remembers about you\n\n"
MAX_CHARS = 12000


class Memory:
    def __init__(self, paths: Paths) -> None:
        self.paths = paths

    def text(self) -> str:
        try:
            return self.paths.memory_file.read_text(encoding="utf-8")
        except OSError:
            return ""

    def facts(self) -> list[str]:
        return [ln[2:].strip() for ln in self.text().splitlines() if ln.startswith("- ")]

    def remember(self, fact: str, added: list[str] | None = None) -> str:
        """Keeps a fact (`added` gets the line written, for Revert in Actions)."""
        fact = " ".join(fact.split()).strip()
        if not fact:
            return "Nothing to remember."
        existing = self.facts()
        if any(fact.lower() == f.split(" (since ")[0].lower() for f in existing):
            return "I already remember that."
        line = f"- {fact} (since {time.strftime('%Y-%m-%d')})\n"
        body = self.text() or HEADER
        if not body.endswith("\n"):
            body += "\n"
        body += line
        if len(body) > MAX_CHARS:  # keep the newest facts if it grows too big
            lines = body.splitlines(keepends=True)
            facts = [ln for ln in lines if ln.startswith("- ")]
            body = HEADER + "".join(facts[-200:])
        atomic_write(self.paths.memory_file, body)
        if added is not None:
            added.append(line.rstrip("\n"))
        return "Got it, I'll remember that."

    def forget(self, text: str, dropped_lines: list[str] | None = None) -> str:
        """Forgets the facts with all of `text`'s words (`dropped_lines` gets the lines taken out, for Revert)."""
        words = [w for w in re.findall(r"\w+", text.lower()) if len(w) > 2]
        if not words:
            return "Tell me what to forget."
        lines = self.text().splitlines(keepends=True)
        keep, dropped = [], []
        for ln in lines:
            low = ln.lower()
            if ln.startswith("- ") and all(w in low for w in words):
                dropped.append(ln[2:].strip())
                if dropped_lines is not None:
                    dropped_lines.append(ln.rstrip("\n"))
            else:
                keep.append(ln)
        if not dropped:
            return "I didn't have anything like that."
        atomic_write(self.paths.memory_file, "".join(keep) or HEADER)
        return f"Forgotten: {'; '.join(dropped)}"

    def drop_lines(self, lines: list[str]) -> list[str]:
        """Takes out exactly these lines (Revert of something remembered); returns those that were there."""
        wanted = {ln.rstrip("\n") for ln in lines}
        keep, gone = [], []
        for ln in self.text().splitlines(keepends=True):
            (gone if ln.rstrip("\n") in wanted else keep).append(ln)
        if gone:
            atomic_write(self.paths.memory_file, "".join(keep) or HEADER)
        return [ln.rstrip("\n") for ln in gone]

    def add_lines(self, lines: list[str]) -> list[str]:
        """Puts back lines taken out (Revert of something forgotten), as they were; returns those added."""
        body = self.text() or HEADER
        have = {ln.rstrip("\n") for ln in body.splitlines()}
        new = [ln.rstrip("\n") for ln in lines if ln.startswith("- ") and ln.rstrip("\n") not in have]
        if new:
            atomic_write(self.paths.memory_file, body + ("" if body.endswith("\n") else "\n") + "\n".join(new) + "\n")
        return new

    def for_prompt(self) -> str:
        facts = self.facts()
        return "\n".join(f"- {f}" for f in facts[-150:])
