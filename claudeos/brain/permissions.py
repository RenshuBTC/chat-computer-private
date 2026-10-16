"""The permission gate: every tool call Claude makes passes through here.

Safe things (reading, looking, showing) just happen. Things that can't be undone, cost money,
send something as you, or change the machine are spoken as a yes/no question first. A few
things are never allowed (reading the sign-in token, wiping disks, running downloaded scripts).
File edits are backed up first so they can be undone.

When the request came from the Claude app on the phone, nobody may be at the PC to hear the
question. Then Claude is told to ask in its reply; when the person answers yes on the phone, the
same action is allowed once when Claude tries it again.
"""
from __future__ import annotations

import asyncio
import json
import logging
import os
import re
import shlex
import time
from pathlib import Path
from typing import Any, Awaitable, Callable

from claude_agent_sdk import PermissionResultAllow, PermissionResultDeny

from ..answers import classify_answer
from ..config import Paths
from ..journal import Journal

log = logging.getLogger("claudeos.permissions")

# A confirm callback speaks the question and returns ("yes"|"no"|"unclear"|"timeout", what the user said),
# or ("phone", "") when the person is on the phone and the question has to go in Claude's reply.
ConfirmFn = Callable[[str], Awaitable[tuple[str, str]]]
PHONE_ASKED = "\x00phone"
PHONE_OK_FOR_S = 10 * 60       # a yes on the phone allows the action for this long
PHONE_QUESTION_FOR_S = 30 * 60 # a question on the phone can be answered for this long
ToolPolicy = Callable[[dict[str, Any]], "str | None"]   # returns a question, or None to allow

READ_ONLY_CMDS = {
    "ls", "cat", "head", "tail", "wc", "grep", "egrep", "fgrep", "rg", "stat", "file", "du", "df", "free",
    "uptime", "date", "cal", "whoami", "id", "hostname", "uname", "lsblk", "lsusb", "lspci", "pwd", "which",
    "type", "echo", "printf", "sort", "uniq", "cut", "tr", "jq", "ps", "pgrep", "basename", "dirname",
    "realpath", "readlink", "md5sum", "sha256sum", "sha1sum", "tree", "nproc", "lscpu", "sensors", "locale",
    "identify", "ffprobe", "true", "false", "cd", "column", "nl", "fold", "rev", "seq", "test", "[", "w",
    "who", "last", "lsmod", "findmnt", "mount", "ip", "ss", "ping", "host", "dig", "nslookup", "tac", "comm",
    "diff", "cmp", "zcat", "xxd", "od", "strings", "getent", "groups", "timedatectl", "hostnamectl",
    "xdpyinfo", "xwininfo", "xprop", "pdfinfo", "exiftool", "mediainfo", "apt-cache", "dpkg-query",
}
SUBCOMMAND_READ_ONLY = {
    "nmcli": [["general"], ["g"], ["device", "status"], ["d", "status"], ["dev", "status"], ["device", "wifi", "list"],
              ["dev", "wifi", "list"], ["d", "wifi", "list"], ["connection", "show"], ["con", "show"], ["c", "show"],
              ["networking", "connectivity"], ["radio"], ["device", "show"], ["dev", "show"]],
    "wpctl": [["status"], ["inspect"], ["get-volume"]],
    "pactl": [["list"], ["info"], ["get-sink-volume"], ["get-sink-mute"], ["get-default-sink"], ["get-default-source"]],
    "bluetoothctl": [["devices"], ["info"], ["show"], ["list"], ["paired-devices"]],
    "systemctl": [["status"], ["is-active"], ["is-enabled"], ["is-failed"], ["list-units"], ["list-unit-files"], ["show"],
                  ["--user", "status"], ["--user", "list-units"]],
    "git": [["status"], ["log"], ["diff"], ["show"], ["branch"], ["remote", "-v"], ["ls-files"]],
    "xrandr": [[], ["--query"], ["-q"], ["--listmonitors"]],
    "wmctrl": [["-l"], ["-d"], ["-m"]],
    "xdotool": [["getactivewindow"], ["getwindowname"], ["search"], ["getmouselocation"], ["getwindowgeometry"]],
    "apt": [["list"], ["search"], ["show"], ["policy"]],
    "dpkg": [["-l"], ["-L"], ["-s"], ["--list"]],
    "journalctl": None,   # read-only unless it vacuums/rotates (checked below)
    "find": None,         # read-only unless -delete/-exec (checked below)
    "curl": None,         # GET without writing files or uploading (checked below)
    "sed": None,          # only -n without in-place editing (checked below)
}
CURL_OK_FLAGS = {"-s", "-S", "-L", "-f", "-I", "-i", "-k?", "--head", "--silent", "--show-error", "--location",
                 "--fail", "--compressed", "-sL", "-sSL", "-fsSL", "-fsS", "-sS", "-Ls", "-sf", "-sI", "-g"}
CURL_VALUE_FLAGS = {"-m", "--max-time", "-H", "--header", "-A", "--user-agent", "--connect-timeout", "-e", "--referer"}

SAFE_REDIRECTS = {">/dev/null", "2>/dev/null", "2>&1", "1>/dev/null", "&>/dev/null"}

FORBIDDEN_PATTERNS = [
    (re.compile(r"(claudeos/secrets|claude_token|claude_login\.json|\.credentials\.json|CLAUDE_CODE_OAUTH_TOKEN|ANTHROPIC_API_KEY)", re.I),
     "That would expose your Claude sign-in token, so ClaudeOS blocks it."),
    (re.compile(r"/proc/[^\s]*/environ"), "That would expose secret settings, so ClaudeOS blocks it."),
    (re.compile(r"(^|[;&|]\s*)(printenv|env|set|export\s+-p|declare\s+-p)\s*($|[;&|])"),
     "Listing all settings would expose your Claude sign-in token, so ClaudeOS blocks it."),
    (re.compile(r"\|\s*(sudo\s+)?(ba|z|da|k|fi)?sh\b|\|\s*(python3?|perl|ruby|node)\b|(ba)?sh\s+<\(\s*(curl|wget)"),
     "ClaudeOS never runs scripts straight from the internet. Download it, look at it, then decide."),
    (re.compile(r"\b(mkfs(\.\w+)?|wipefs|fdisk|sfdisk|sgdisk|gdisk|parted|cfdisk)\b"),
     "Changing disk partitions could destroy Windows or ClaudeOS, so it isn't allowed."),
    (re.compile(r"\bdd\b[^|;&]*\bof=/dev/"), "Writing straight to a disk isn't allowed."),
    (re.compile(r">\s*/dev/(sd|nvme|hd|vd|mmcblk)"), "Writing straight to a disk isn't allowed."),
    (re.compile(r"\brm\s+(-[a-zA-Z]*\s+)*(-[a-zA-Z]*r[a-zA-Z]*|--recursive)\b[^;&|]*\s(/|~|~/|\$HOME|/home|/mnt/windows|/etc|/usr|/boot|/opt|/var)/?(\s|$|\*)"),
     "That would delete a whole system folder, so ClaudeOS blocks it."),
    (re.compile(r"--no-preserve-root"), "That isn't allowed."),
    (re.compile(r":\(\)\s*\{\s*:\|:&\s*\};:"), "That isn't allowed."),
    (re.compile(r"\bsudo\b(?!\s+-n\s+/usr/local/libexec/claudeos/claudeos-root)"),
     "Admin commands go through ClaudeOS's own tools (install_apps, update_system, power) instead of sudo."),
    (re.compile(r"\bchmod\s+(-R\s+)?[0-7]*7[0-7]*\s+/(\s|$)|\bchown\s+-R\s+\S+\s+/(\s|$)"), "That isn't allowed."),
]


def _split_segments(command: str) -> list[list[str]] | None:
    """Tokenize a shell command into simple-command segments. None if it uses features we
    don't analyse (substitutions, here-docs, subshells, background jobs)."""
    if any(x in command for x in ("$(", "`", "<(", ">(", "<<")):
        return None
    try:
        lex = shlex.shlex(command, posix=True, punctuation_chars=";&|<>()")
        lex.whitespace_split = True
        tokens = list(lex)
    except ValueError:
        return None
    segments: list[list[str]] = [[]]
    i = 0
    while i < len(tokens):
        t = tokens[i]
        if t in ("|", "||", "&&", ";", ";;"):
            segments.append([])
        elif t in ("(", ")", "&"):
            return None
        elif t in (">", ">>", "<", ">&", "&>", "2>", "1>") or (t and set(t) <= set("<>&")):
            # Only harmless redirections: to /dev/null or 2>&1.
            nxt = tokens[i + 1] if i + 1 < len(tokens) else ""
            prev = segments[-1][-1] if segments[-1] else ""
            red = (prev if prev in ("2", "1") else "") + t + nxt
            if prev in ("2", "1") and segments[-1]:
                segments[-1].pop()
            if red.replace(" ", "") not in SAFE_REDIRECTS and t + nxt not in SAFE_REDIRECTS:
                return None
            i += 2
            continue
        else:
            segments[-1].append(t)
        i += 1
    return [s for s in segments if s]


def _segment_read_only(seg: list[str]) -> bool:
    # Leading VAR=value assignments make the segment not read-only for our purposes.
    if not seg or "=" in seg[0] and not seg[0].startswith("-"):
        return False
    cmd = os.path.basename(seg[0])
    args = seg[1:]
    if cmd in READ_ONLY_CMDS:
        if cmd == "date" and any(a in ("-s", "--set") for a in args):
            return False
        if cmd == "hostname" and args and not args[0].startswith("-"):
            return False
        if cmd == "mount" and args:
            return False
        if cmd == "ip" and any(a in ("add", "del", "delete", "set", "flush", "change", "replace") for a in args):
            return False
        if cmd in ("timedatectl", "hostnamectl") and args and args[0].startswith("set"):
            return False
        return True
    if cmd not in SUBCOMMAND_READ_ONLY:
        return False
    if cmd == "find":
        return not any(a in ("-delete", "-exec", "-execdir", "-ok", "-okdir", "-fprint", "-fprint0", "-fprintf", "-fls")
                       for a in args)
    if cmd == "journalctl":
        return not any(a.startswith(("--vacuum", "--rotate", "--flush", "--sync", "--relinquish")) for a in args)
    if cmd == "sed":
        return "-n" in args and not any(a.startswith(("-i", "--in-place")) for a in args) and \
            not any(re.search(r"(^|;)\s*[wWe]\b|/[wWe]\s", a) for a in args if not a.startswith("-"))
    if cmd == "curl":
        j = 0
        while j < len(args):
            a = args[j]
            if a in CURL_VALUE_FLAGS:
                j += 2
                continue
            if a.startswith("-") and a not in CURL_OK_FLAGS:
                return False
            if not a.startswith("-") and not re.match(r"^https?://", a):
                return False
            j += 1
        return True
    allowed = SUBCOMMAND_READ_ONLY[cmd] or []
    # Strip global flags like `nmcli -t -f FIELDS` before matching the subcommand.
    rest: list[str] = []
    skip = False
    for a in args:
        if skip:
            skip = False
            continue
        if cmd == "nmcli" and a in ("-f", "--fields", "-e", "--escape", "-c", "--colors", "-w", "--wait"):
            skip = True
            continue
        if a.startswith("-") and cmd in ("nmcli",):
            continue
        rest.append(a)
    for pattern in allowed:
        if rest[: len(pattern)] == pattern:
            if cmd == "nmcli" and pattern == ["radio"] and any(x in ("on", "off") for x in rest):
                return False
            if cmd == "git" and pattern == ["branch"] and any(x in ("-d", "-D", "-m", "-M", "--delete") for x in rest):
                return False
            return True
    return False


def classify_bash(command: str) -> tuple[str, str]:
    """('allow'|'ask'|'deny', reason)."""
    c = command.strip()
    for pat, why in FORBIDDEN_PATTERNS:
        if pat.search(c):
            return "deny", why
    segs = _split_segments(c)
    if segs is not None and segs and all(_segment_read_only(s) for s in segs):
        return "allow", "read-only"
    return "ask", "changes something"


SENSITIVE_READ = [re.compile(p) for p in (
    r"/\.ssh(/|$)", r"/\.gnupg(/|$)", r"/\.engine/\.credentials\.json$", r"/etc/shadow", r"/etc/sudoers",
    r"/\.local/share/keyrings(/|$)", r"/\.mozilla/.*/(logins|key\d)\.", r"/\.config/(google-chrome|chromium)/.*/Login Data",
    r"/claude_token$", r"/claude_login\.json$",   # sign-ins left on the Windows drive by ClaudeOS Setup
)]


class PermissionGate:
    def __init__(self, paths: Paths, journal: Journal, confirm: ConfirmFn | None,
                 tool_policies: dict[str, ToolPolicy] | None = None,
                 on_decision: Callable[[str, str, dict[str, Any]], None] | None = None) -> None:
        self.paths = paths
        self.journal = journal
        self.confirm = confirm
        self.tool_policies = tool_policies or {}
        self.on_decision = on_decision
        self._lock = asyncio.Lock()
        self._phone_question: dict[str, Any] | None = None   # asked in the chat, waiting for an answer
        self._phone_ok: dict[str, float] = {}                 # action key -> allowed until

    # ------------------------------------------------------------------ helpers
    def _path_denied(self, path: str | None) -> str | None:
        if not path:
            return None
        p = os.path.realpath(os.path.expanduser(path))
        if self.paths.is_secret(p) or any(r.search(p) for r in SENSITIVE_READ):
            return "That file holds passwords or sign-in keys, so ClaudeOS keeps it private."
        return None

    async def _ask(self, question: str, key: str | None = None) -> tuple[bool, str]:
        key = key or question
        until = self._phone_ok.pop(key, 0.0)
        if until > time.time():
            return True, "yes (on their phone)"
        if self.confirm is None:
            return False, ""
        async with self._lock:  # one spoken question at a time
            verdict, said = await self.confirm(question)
        if verdict == "phone":
            self._phone_question = {"key": key, "question": question, "at": time.time()}
            return False, PHONE_ASKED
        return verdict == "yes", said

    def phone_reply(self, text: str) -> None:
        """What the person typed on the phone, in case it answers the question Claude asked there."""
        q = self._phone_question
        if not q or time.time() - q["at"] > PHONE_QUESTION_FOR_S:
            return
        verdict = classify_answer(text)
        if verdict == "yes":
            self._phone_ok[q["key"]] = time.time() + PHONE_OK_FOR_S
            self._phone_question = None
        elif verdict == "no":
            self._phone_question = None

    @staticmethod
    def _key(tool: str, inp: dict[str, Any]) -> str:
        if tool == "Bash":
            return "Bash:" + str(inp.get("command", "")).strip()
        try:
            return tool + ":" + json.dumps(inp, sort_keys=True, default=str)
        except (TypeError, ValueError):
            return tool + ":" + repr(inp)

    def _note(self, decision: str, tool: str, inp: dict[str, Any]) -> None:
        if self.on_decision:
            try:
                self.on_decision(decision, tool, inp)
            except Exception:
                log.exception("on_decision failed")

    @staticmethod
    def _deny(msg: str) -> PermissionResultDeny:
        return PermissionResultDeny(message=msg, interrupt=False)

    def _user_said_no(self, said: str) -> PermissionResultDeny:
        if said == PHONE_ASKED:
            q = (self._phone_question or {}).get("question", "Should I go ahead?")
            return self._deny("Not done yet: this needs their OK, and they're messaging you from the Claude app on "
                              f'their phone, so nobody is at the PC to answer. Ask them in your reply: "{q}" Then '
                              "stop. If they answer yes, do exactly the same thing again and it will be allowed.")
        if said:
            return self._deny(f'The user did not approve. They said: "{said}". Do not retry this action; '
                              f"follow what they said or ask what they'd like instead.")
        return self._deny("The user did not answer, so this was not done. Mention it briefly and move on.")

    # ------------------------------------------------------------------ the gate
    async def __call__(self, tool: str, inp: dict[str, Any], ctx: Any = None):
        try:
            return await self._decide(tool, inp)
        except Exception as exc:  # never let a bug in the gate approve something by accident
            log.exception("permission gate error for %s", tool)
            return self._deny(f"ClaudeOS couldn't check this action ({exc}).")

    async def _decide(self, tool: str, inp: dict[str, Any]):
        if tool in ("Read", "Glob", "Grep", "LS", "NotebookRead"):
            why = self._path_denied(inp.get("file_path") or inp.get("path") or inp.get("notebook_path"))
            if why:
                self._note("deny", tool, inp)
                return self._deny(why)
            return PermissionResultAllow(updated_input=inp)

        if tool in ("WebSearch", "WebFetch", "TodoWrite", "ToolSearch"):
            return PermissionResultAllow(updated_input=inp)

        if tool in ("Write", "Edit", "MultiEdit", "NotebookEdit"):
            path = inp.get("file_path") or inp.get("notebook_path") or ""
            why = self._path_denied(path)
            if why or os.path.realpath(path).startswith("/opt/claudeos"):
                self._note("deny", tool, inp)
                return self._deny(why or "ClaudeOS's own program files can't be edited from here.")
            existed = os.path.isfile(path)
            backup = await asyncio.to_thread(self.journal.backup, path) if existed else None
            self.journal.record_overwrite(path, backup, "changed" if existed else "created")
            return PermissionResultAllow(updated_input=inp)

        if tool == "Bash":
            cmd = str(inp.get("command", ""))
            verdict, why = classify_bash(cmd)
            if verdict == "deny":
                self._note("deny", tool, inp)
                return self._deny(why)
            if verdict == "allow":
                return PermissionResultAllow(updated_input=inp)
            desc = (inp.get("description") or "").strip().rstrip(".")
            question = (f"Can I run a command to {desc[0].lower() + desc[1:]}?" if desc and len(desc) > 3
                        else "Can I run this command: " + _short(cmd) + "?")
            ok, said = await self._ask(question, self._key(tool, inp))
            self._note("allow" if ok else "deny", tool, inp)
            if ok:
                self.journal.record("command", desc or _short(cmd), command=cmd)
                return PermissionResultAllow(updated_input=inp)
            return self._user_said_no(said)

        if tool.startswith("mcp__claudeos__"):
            name = tool[len("mcp__claudeos__"):]
            policy = self.tool_policies.get(name)
            question = policy(inp) if policy else None
            if question and question.startswith("DENY:"):
                self._note("deny", tool, inp)
                return self._deny(question[5:].strip())
            if not question:
                return PermissionResultAllow(updated_input=inp)
            ok, said = await self._ask(question, self._key(tool, inp))
            self._note("allow" if ok else "deny", tool, inp)
            return PermissionResultAllow(updated_input=inp) if ok else self._user_said_no(said)

        # Any other tool (e.g. one added by a future engine version): ask.
        ok, said = await self._ask(f"Can I use {tool}?", self._key(tool, inp))
        return PermissionResultAllow(updated_input=inp) if ok else self._user_said_no(said)


def _short(cmd: str, n: int = 80) -> str:
    cmd = " ".join(cmd.split())
    return cmd if len(cmd) <= n else cmd[: n - 1] + "…"
