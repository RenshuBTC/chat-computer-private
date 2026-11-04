"""The permission gate: every tool call the model makes passes through here.

Safe things (reading, looking, showing) just happen. Things that can't be undone, cost money,
send something as you, or change the machine are spoken as a yes/no question first. A few
things are never allowed (reading the sign-ins Chat Computer keeps, wiping disks, running downloaded scripts).
File edits are backed up first so they can be undone.

A command runs without asking only when every part of it is known to only read. Anything this file
can't analyse with certainty (substitutions, unusual redirections, several lines) is asked about.
"""
from __future__ import annotations

import asyncio
import glob
import itertools
import logging
import os
import re
import shlex
import shutil
from dataclasses import dataclass
from typing import Any, Awaitable, Callable

from .. import update
from ..config import Paths
from ..journal import Journal

log = logging.getLogger("chatcomputer.permissions")


@dataclass
class PermissionResultAllow:
    """Go ahead, with this input."""
    updated_input: dict[str, Any] | None = None
    behavior: str = "allow"


@dataclass
class PermissionResultDeny:
    """Not done: the message says why (the model reads it as the tool's result)."""
    message: str = ""
    interrupt: bool = False
    behavior: str = "deny"

# A confirm callback asks the question and returns ("yes"|"no"|"unclear"|"timeout", what the user said).
ConfirmFn = Callable[[str], Awaitable[tuple[str, str]]]
ToolPolicy = Callable[[dict[str, Any]], "str | None"]   # returns a question, or None to allow

READ_ONLY_CMDS = {
    "ls", "cat", "head", "tail", "wc", "grep", "egrep", "fgrep", "rg", "stat", "file", "du", "df", "free",
    "uptime", "date", "cal", "whoami", "id", "hostname", "uname", "lsblk", "lsusb", "lspci", "pwd", "which",
    "type", "echo", "printf", "sort", "uniq", "cut", "tr", "jq", "ps", "pgrep", "basename", "dirname",
    "realpath", "readlink", "md5sum", "sha256sum", "sha1sum", "tree", "nproc", "lscpu", "sensors", "locale",
    "identify", "ffprobe", "true", "false", "cd", "column", "nl", "fold", "rev", "seq", "test", "[", "w",
    "who", "last", "lsmod", "findmnt", "mount", "ip", "ss", "ping", "host", "dig", "nslookup", "tac", "comm",
    "diff", "cmp", "zcat", "xxd", "od", "strings", "getent", "groups", "timedatectl", "hostnamectl",
    "xdpyinfo", "xwininfo", "xprop", "pdfinfo", "mediainfo", "apt-cache", "dpkg-query",
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
    "apt": [["list"], ["search"], ["show"], ["policy"]],
    "dpkg": [["-l"], ["-L"], ["-s"], ["--list"]],
}
# Not on these lists, so always asked about: git (a folder's own git settings can make even "git status" run
# a program) and exiftool (it writes tags, and runs Perl in -if and -p).
CURL_OK_FLAGS = {"-s", "-S", "-L", "-f", "-I", "-i", "--head", "--silent", "--show-error", "--location",
                 "--fail", "--compressed", "-sL", "-sSL", "-fsSL", "-fsS", "-sS", "-Ls", "-sf", "-sI", "-g"}
CURL_VALUE_FLAGS = {"-m", "--max-time", "-H", "--header", "-A", "--user-agent", "--connect-timeout", "-e", "--referer"}

SAFE_REDIRECTS = {">/dev/null", "2>/dev/null", "2>&1", "1>/dev/null", "&>/dev/null", ">&2", "1>&2"}

FORBIDDEN_PATTERNS = [
    (re.compile(r"(chatcomputer/secrets|claude_token|claude_login\.json|\.credentials\.json|CLAUDE_CODE_OAUTH_TOKEN|"
                r"ANTHROPIC_API_KEY|(?<![\w.-])\.engine\b|chatcomputer/web\b)", re.I),
     "That would expose the sign-ins Chat Computer keeps, so Chat Computer blocks it."),
    (re.compile(r"\benviron\b", re.I), "That would expose secret settings, so Chat Computer blocks it."),
    (re.compile(r"[%$]ENV\b|process\.env\b|\bjq\b[^|;&]*\benv\b"),
     "That would expose secret settings, so Chat Computer blocks it."),
    (re.compile(r"(^|[;&|]\s*)(printenv|env|set|export\s+-p|declare\s+-p)\s*($|[;&|])"),
     "Listing all settings could expose secret ones, so Chat Computer blocks it."),
    (re.compile(r"\|\s*(sudo\s+)?(ba|z|da|k|fi)?sh\b|\|\s*(python3?|perl|ruby|node)\b|(ba)?sh\s+<\(\s*(curl|wget)"),
     "Chat Computer never runs scripts straight from the internet. Download it, look at it, then decide."),
    (re.compile(r"\b(mkfs(\.\w+)?|wipefs|fdisk|sfdisk|sgdisk|gdisk|parted|cfdisk)\b"),
     "Changing disk partitions could destroy Windows or Chat Computer, so it isn't allowed."),
    (re.compile(r"\bdd\b[^|;&]*\bof=/dev/"), "Writing straight to a disk isn't allowed."),
    (re.compile(r">\s*/dev/(sd|nvme|hd|vd|mmcblk)"), "Writing straight to a disk isn't allowed."),
    (re.compile(r"\brm\s+(-[a-zA-Z]*\s+)*(-[a-zA-Z]*r[a-zA-Z]*|--recursive)\b[^;&|]*\s(/|~|~/|\$HOME|/home|/mnt/windows|/etc|/usr|/boot|/opt|/var)/?(\s|$|\*)"),
     "That would delete a whole system folder, so Chat Computer blocks it."),
    (re.compile(r"--no-preserve-root"), "That isn't allowed."),
    (re.compile(r":\(\)\s*\{\s*:\|:&\s*\};:"), "That isn't allowed."),
    (re.compile(r"\bsudo\b(?!\s+-n\s+/usr/local/libexec/chatcomputer/chatcomputer-root)"),
     "Admin commands go through Chat Computer's own tools instead of sudo (software installs and updates "
     "programs; drives, printer, display, power and set_timezone do the rest)."),
    (re.compile(r"\bchmod\s+(-R\s+)?[0-7]*7[0-7]*\s+/(\s|$)|\bchown\s+-R\s+\S+\s+/(\s|$)"), "That isn't allowed."),
]

_SEPARATORS = ("|", "||", "&&", ";", ";;")
_PUNCTUATION = set(";&|<>()")


def _split_segments(command: str) -> list[list[str]] | None:
    """Tokenize a shell command into simple-command segments. None if it uses features we
    don't analyse (substitutions, here-docs, subshells, background jobs, several lines)."""
    if any(x in command for x in ("$(", "`", "<(", ">(", "<<", "\n", "\r")):
        return None
    try:
        lex = shlex.shlex(command, posix=True, punctuation_chars=";&|<>()")
        lex.whitespace_split = True
        # shlex would treat # as a comment anywhere, even inside a word, where the shell doesn't: everything
        # after it would go unchecked. As plain text, it is checked like the rest.
        lex.commenters = ""
        tokens = list(lex)
    except ValueError:
        return None
    segments: list[list[str]] = [[]]
    i = 0
    while i < len(tokens):
        t = tokens[i]
        if t in _SEPARATORS:
            segments.append([])
        elif t in ("(", ")", "&"):
            return None
        elif t in (">", ">>", "<", ">&", "&>", "2>", "1>") or (t and set(t) <= set("<>&")):
            # Only harmless redirections: to /dev/null, or between the two outputs.
            nxt = tokens[i + 1] if i + 1 < len(tokens) else ""
            prev = segments[-1][-1] if segments[-1] else ""
            red = (prev if prev in ("2", "1") else "") + t + nxt
            if prev in ("2", "1") and segments[-1]:
                segments[-1].pop()
            if red.replace(" ", "") not in SAFE_REDIRECTS and t + nxt not in SAFE_REDIRECTS:
                return None
            i += 2
            continue
        elif t and set(t) <= _PUNCTUATION:
            return None      # |& and >| and the like: the shell reads them in ways this doesn't follow
        else:
            segments[-1].append(t)
        i += 1
    return [s for s in segments if s]


# ---------------------------------------------------------------------- commands that read, unless...
def _positional(args: list[str], takes_value: set[str]) -> list[str]:
    """The arguments that aren't options or the values of options."""
    out: list[str] = []
    skip = False
    for a in args:
        if skip:
            skip = False
        elif a in takes_value:
            skip = True
        elif not a.startswith("-") or a == "-":
            out.append(a)
    return out


def _short_has(args: list[str], letters: str) -> bool:
    """Whether a group of one-letter options (like -no) includes one of `letters`."""
    return any(a.startswith("-") and not a.startswith("--") and set(a[1:]) & set(letters) for a in args)


def _ps(args: list[str]) -> bool:
    # BSD-style options (no dash) with an e show every program's settings, the sign-in token among them.
    values = {"-p", "-C", "-u", "-U", "-g", "-G", "-t", "-o", "-O", "-q", "-s", "-k", "--pid", "--ppid", "--user",
              "--group", "--sort", "--format", "--tty", "--sid", "--cols", "--columns", "--width", "--rows", "--lines"}
    return not any("e" in a for a in _positional(args, values))


def _sort(args: list[str]) -> bool:
    # -o writes the result into a file; --compress-program runs a program.
    return not (_short_has(args, "o") or any(a.startswith(("--output", "--compress-program")) for a in args))


def _uniq(args: list[str]) -> bool:
    # A second file name is the file uniq writes to.
    return len(_positional(args, {"-f", "-s", "-w"})) <= 1


def _xxd(args: list[str]) -> bool:
    # -r turns a dump back into a file; a second file name is the file xxd writes to.
    if any(a.startswith("-r") for a in args):
        return False
    values = {"-c", "-cols", "-g", "-groupsize", "-l", "-len", "-o", "-offset", "-s", "-seek", "-n", "-name", "-R"}
    return len(_positional(args, values)) <= 1


def _tree(args: list[str]) -> bool:
    # -o writes the listing into a file; -R writes one into every folder.
    return not (_short_has(args, "oR") or any(a.startswith("--output") for a in args))


def _ffprobe(args: list[str]) -> bool:
    return not any(a in ("-o", "-report") for a in args)       # both write files


def _file(args: list[str]) -> bool:
    return not (_short_has(args, "C") or "--compile" in args)  # -C writes a compiled magic file


def _rg(args: list[str]) -> bool:
    return not any(a.startswith("--pre") for a in args)        # --pre runs a program on every file


def _xprop(args: list[str]) -> bool:
    return not any(a in ("-set", "-remove") for a in args)


def _mediainfo(args: list[str]) -> bool:
    return not any("logfile" in a.lower() for a in args)       # --LogFile writes a file


def _ss(args: list[str]) -> bool:
    return not (_short_has(args, "K") or "--kill" in args)     # -K closes connections


def _date(args: list[str]) -> bool:
    return not any(a == "-s" or a.startswith("--set") or re.match(r"^-[uR]*s", a) for a in args)


def _hostname(args: list[str]) -> bool:
    return not (args and not args[0].startswith("-")) and not any(a in ("-F", "--file") for a in args)


def _mount(args: list[str]) -> bool:
    return not args


def _ip(args: list[str]) -> bool:
    # -batch (or -b) runs the commands in a file, which can change any of it.
    if any(a in ("-b", "-batch") or a.startswith(("-b=", "-batch=")) for a in args):
        return False
    return not any(a in ("add", "del", "delete", "set", "flush", "change", "replace", "exec") for a in args)


def _ctl(args: list[str]) -> bool:
    # timedatectl and hostnamectl: only the questions, not set-time, set-timezone, hostname NAME...
    queries = {"status", "show", "show-timesync", "timesync-status", "list-timezones"}
    return all(a in queries for a in _positional(args, {"-p", "--property", "-H", "--host", "-M", "--machine"}))


def _find(args: list[str]) -> bool:
    return not any(a in ("-delete", "-exec", "-execdir", "-ok", "-okdir", "-fprint", "-fprint0", "-fprintf", "-fls")
                   for a in args)


def _journalctl(args: list[str]) -> bool:
    return not any(a.startswith(("--vacuum", "--rotate", "--flush", "--sync", "--relinquish", "--setup-keys",
                                 "--update-catalog", "--cursor-file")) for a in args)


_SED_ADDRESS = r"(\d+|\$|/[^/\\]*/)"
_SED_PRINT = re.compile(rf"^\s*({_SED_ADDRESS}(\s*,\s*{_SED_ADDRESS})?)?\s*p"
                        rf"(\s*;\s*({_SED_ADDRESS}(\s*,\s*{_SED_ADDRESS})?)?\s*p)*\s*;?\s*$")


def _sed(args: list[str]) -> bool:
    # Only scripts that print lines (sed -n '10,20p', '/error/p'): sed scripts can also write files (w) and run
    # programs (e), in more ways than are worth recognizing.
    scripts: list[str] = []
    rest: list[str] = []
    i = 0
    while i < len(args):
        a = args[i]
        if a in ("-e", "--expression"):
            if i + 1 >= len(args):
                return False
            scripts.append(args[i + 1])
            i += 1
        elif a.startswith("--expression="):
            scripts.append(a.split("=", 1)[1])
        elif a in ("--quiet", "--silent", "--regexp-extended", "--separate", "--null-data", "--unbuffered", "--posix",
                   "--"):
            pass
        elif a.startswith("-") and a != "-":
            if a.startswith("--") or not set(a[1:]) <= set("nErszu"):
                return False   # -i (change the file), -f (a script file) and anything else
        else:
            rest.append(a)
        i += 1
    if not scripts:
        if not rest:
            return False
        scripts.append(rest[0])
    return all(_SED_PRINT.match(s) for s in scripts)


def _curl(args: list[str]) -> bool:
    # A plain download to the screen: no files written, nothing sent.
    j = 0
    while j < len(args):
        a = args[j]
        if a in CURL_VALUE_FLAGS:
            if j + 1 < len(args) and args[j + 1].startswith("@"):
                return False       # -H @file sends a file's contents
            j += 2
            continue
        if a.startswith("-") and a not in CURL_OK_FLAGS:
            return False
        if not a.startswith("-") and not re.match(r"^https?://", a):
            return False
        j += 1
    return True


_XDOTOOL_QUESTIONS = {"getactivewindow", "getwindowname", "getmouselocation", "getwindowgeometry", "getwindowpid",
                      "getwindowfocus", "getdisplaygeometry", "search"}


def _xdotool(args: list[str]) -> bool:
    # xdotool runs several commands in a row (search ... exec ...): every word must be a question, a window
    # number, or what search looks for.
    pattern_next = False
    for a in args:
        if a.startswith("-"):
            continue
        if pattern_next:
            pattern_next = False
            continue
        if a in _XDOTOOL_QUESTIONS:
            pattern_next = a == "search"
            continue
        if re.fullmatch(r"%@|%-?\d+|\d+|0x[0-9a-fA-F]+", a):
            continue
        return False
    return True


def _wmctrl(args: list[str]) -> bool:
    # Listing windows (-l), desktops (-d) or the window manager (-m), with details (-p -G -x).
    return bool(args) and all(re.fullmatch(r"-[ldmpGx]+", a) for a in args)


def _xrandr(args: list[str]) -> bool:
    return all(a in ("--query", "-q", "--listmonitors", "--listactivemonitors", "--current", "--verbose", "--prop",
                     "--properties", "--listproviders") for a in args)


_SPECIAL: dict[str, Callable[[list[str]], bool]] = {
    "ps": _ps, "sort": _sort, "uniq": _uniq, "xxd": _xxd, "tree": _tree, "ffprobe": _ffprobe, "file": _file,
    "rg": _rg, "xprop": _xprop, "mediainfo": _mediainfo, "ss": _ss, "date": _date, "hostname": _hostname,
    "mount": _mount, "ip": _ip, "timedatectl": _ctl, "hostnamectl": _ctl, "find": _find, "journalctl": _journalctl,
    "sed": _sed, "curl": _curl, "xdotool": _xdotool, "wmctrl": _wmctrl, "xrandr": _xrandr,
}


# Where the system keeps its own programs. A program of the same name anywhere else (./ls, /tmp/x/cat, or one
# found first on a PATH with a folder of the person's in it) isn't the one on the read-only list: it may do anything.
SYSTEM_BIN_DIRS = ("/bin", "/usr/bin", "/sbin", "/usr/sbin", "/usr/local/bin", "/usr/local/sbin")


def _system_program(word: str) -> bool:
    """Whether a command word names a program the system keeps: one found on PATH, or one in a system folder."""
    if "/" in word:
        path = os.path.realpath(os.path.expanduser(word))
    else:
        found = shutil.which(word)
        if not found:
            return True         # not installed here, so it can't run: the read-only list still applies to it
        path = os.path.realpath(found)
    return any(path.startswith(d + "/") for d in SYSTEM_BIN_DIRS)


def _segment_read_only(seg: list[str]) -> bool:
    # Leading VAR=value assignments make the segment not read-only for our purposes.
    if not seg or "=" in seg[0] and not seg[0].startswith("-"):
        return False
    if not _system_program(seg[0]):
        return False            # a program written somewhere else, whatever it's called
    cmd = os.path.basename(seg[0])
    args = seg[1:]
    if cmd in _SPECIAL:
        return _SPECIAL[cmd](args)
    if cmd in READ_ONLY_CMDS:
        return True
    if cmd not in SUBCOMMAND_READ_ONLY:
        return False
    if cmd == "nmcli" and any(a.startswith("-") and a.lstrip("-").split("=")[0] and
                              "show-secrets".startswith(a.lstrip("-").split("=")[0]) for a in args):
        return False       # -s / --show-secrets shows Wi-Fi passwords
    # Strip global flags like `nmcli -t -f FIELDS` before matching the subcommand.
    rest: list[str] = []
    skip = False
    for a in args:
        if skip:
            skip = False
            continue
        if cmd == "nmcli" and a in ("-f", "--fields", "-g", "--get-values", "-e", "--escape", "-c", "--colors", "-w",
                                    "--wait"):
            skip = True
            continue
        if a.startswith("-") and cmd in ("nmcli",):
            continue
        rest.append(a)
    for pattern in SUBCOMMAND_READ_ONLY[cmd]:
        if pattern and rest[: len(pattern)] == pattern:
            if cmd == "nmcli" and pattern == ["radio"] and any(x in ("on", "off") for x in rest):
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


PROGRAM_DENIED = ("Those are Chat Computer's own program files, so they can't be changed from here. To update Chat Computer, "
                  "the person puts a new version in the folder they ran Chat Computer Setup from, on Windows; Chat Computer "
                  "offers it when it starts.")

SENSITIVE_READ = [re.compile(p) for p in (
    r"/\.ssh(/|$)", r"/\.gnupg(/|$)", r"/\.engine/\.credentials\.json$", r"/etc/shadow", r"/etc/sudoers",
    r"/\.local/share/keyrings(/|$)", r"/\.mozilla/.*/(logins|key\d)\.", r"/\.config/(google-chrome|chromium)/.*/Login Data",
    r"/claude_token$", r"/claude_login\.json$",   # sign-ins left on the Windows drive by Chat Computer Setup
    r"^/proc/.*/environ$",                         # a program's settings: the engine's hold the sign-in
)]

# Files the computer runs commands from: changing one changes what happens later, so it's asked about.
_RUNS_CODE_DIRS = (".config/autostart/", ".config/systemd/", ".config/environment.d/", ".local/bin/", "bin/")


def _inside(path: str, folder: str) -> bool:
    folder = folder.rstrip("/")
    return path == folder or path.startswith(folder + "/") or folder == ""


def _resolve(path: str) -> str:
    """A tool's path as the engine reads it: ~ expanded, relative to the home folder (the engine works there),
    links followed."""
    p = os.path.expanduser(str(path))
    if not os.path.isabs(p):
        p = os.path.join(os.path.expanduser("~"), p)
    return os.path.realpath(p)


def private_dirs(paths: Paths) -> list[str]:
    """Chat Computer's own private folders: the sign-ins, the engine's settings, the built-in browser's logins, and the
    sign-ins of the sites Messages reads."""
    return [os.path.realpath(p) for p in (paths.secrets, paths.engine, paths.web, paths.sources)]


def is_private(paths: Paths, path: str) -> bool:
    """Whether `path` is inside one of Chat Computer's private folders, or is another file that holds passwords or
    sign-in keys."""
    p = _resolve(path)
    return (paths.is_secret(p) or any(_inside(p, d) for d in private_dirs(paths))
            or any(r.search(p) for r in SENSITIVE_READ))


def holds_private(paths: Paths, path: str) -> bool:
    """Whether the folder `path` contains one of Chat Computer's private folders (the home folder does)."""
    r = _resolve(path)
    return any(_inside(d, r) for d in private_dirs(paths))


def program_dirs() -> list[str]:
    """Chat Computer's own program: where it's installed, and the folder on Windows' drive it updates itself from
    (update.py). The model can read them but not change them: what's in them is Chat Computer, this gate included."""
    out = [os.path.realpath(update.APP.parent)]
    src = update.source_dir()
    if src is not None:
        out.append(os.path.realpath(src))
    return out


def is_program(path: str) -> bool:
    """Whether `path` is part of Chat Computer's own program (program_dirs). Windows' drive doesn't mind the case of
    names, so neither does this."""
    p = _resolve(path).casefold()
    return any(_inside(p, d.casefold()) for d in program_dirs())


def holds_program(path: str) -> bool:
    """Whether the folder `path` contains Chat Computer's own program."""
    r = _resolve(path).casefold()
    return any(_inside(d.casefold(), r) for d in program_dirs())


def _shell_expand(arg: str, cwd: str) -> list[str]:
    """What the shell makes of one word: ~ and $HOME expanded, relative to `cwd`, wildcards matched."""
    home = os.path.expanduser("~")
    a = home + arg[1:] if arg == "~" or arg.startswith("~/") else arg
    a = a.replace("${HOME}", home).replace("$HOME", home)
    if not os.path.isabs(a):
        a = os.path.join(cwd, a)
    if any(c in a for c in "*?["):
        return glob.glob(a) or [a]
    return [a]


_GREP_VALUES = {"-e", "--regexp", "-f", "--file", "-m", "--max-count", "-A", "--after-context", "-B",
                "--before-context", "-C", "--context", "-d", "--directories", "-D", "--devices", "--label"}
_RG_VALUES = {"-e", "--regexp", "-f", "--file", "-g", "--glob", "--iglob", "-t", "--type", "-T", "--type-not",
              "-m", "--max-count", "-A", "-B", "-C", "-j", "--threads", "-M", "--max-columns", "--max-depth", "-d",
              "--type-add", "-E", "--encoding", "--sort", "--sortr", "-r", "--replace", "--max-filesize"}


def _cd_targets(command: str, starts: list[str]) -> set[str]:
    """The folders a command's cd moves to (the engine keeps that folder for its next commands)."""
    out: set[str] = set()
    for start in starts:
        cwd = start
        for seg in _split_segments(command) or []:
            if os.path.basename(seg[0]) in ("cd", "pushd"):
                cwd = _shell_expand(seg[1] if len(seg) > 1 else "~", cwd)[0]
                out.add(os.path.normpath(cwd))
    return out


def _reaches_private(paths: Paths, command: str, cwds: set[str] | frozenset[str] = frozenset()) -> bool:
    """Whether a command names, matches with wildcards, or searches through Chat Computer's private files, run from the
    home folder, Chat Computer's own folder, or any folder an earlier command moved to (`cwds`)."""
    segs = _split_segments(command)
    if not segs:
        return False
    for start in (os.path.expanduser("~"), str(paths.root), *sorted(cwds)):
        cwd = start
        for seg in segs:
            name, args = os.path.basename(seg[0]), seg[1:]
            if name in ("cd", "pushd"):
                targets = _shell_expand(args[0] if args else "~", cwd)
                if any(is_private(paths, t) for t in targets):
                    return True
                cwd = targets[0]
                continue
            grep = name in ("grep", "egrep", "fgrep", "rg")
            words = _positional(args, (_RG_VALUES if name == "rg" else _GREP_VALUES) if grep else set())
            if any(is_private(paths, p) for w in words for p in _shell_expand(w, cwd)):
                return True
            if not grep:
                continue
            recursive = name == "rg" or any(
                a in ("--recursive", "--dereference-recursive") or a.startswith("--directories=recurse")
                or (a.startswith("-") and not a.startswith("--") and set(a[1:]) & set("rR")) for a in args) \
                or any(a == "-d" and b == "recurse" for a, b in itertools.pairwise(args))
            if recursive:
                patterned = any(a in ("-e", "--regexp", "-f", "--file") or a.startswith(("--regexp=", "--file="))
                                for a in args)
                roots = words if patterned else words[1:]
                dirs = [p for w in roots for p in _shell_expand(w, cwd) if os.path.isdir(p)] if roots else [cwd]
                if any(holds_private(paths, d) for d in dirs):
                    return True
    return False


def _changes_update_source(paths: Paths, command: str) -> bool:
    """Whether a command (one that changes something) names the folder Chat Computer updates itself from, or a file
    in it, or moves into it. Chat Computer's installed program is root's, so commands can't change it anyway; this
    folder is on Windows' drive, which commands can write to."""
    src = update.source_dir()
    if src is None:
        return False
    real = os.path.realpath(src).casefold()
    low = command.casefold()
    # Named in the command as it's written (also inside quotes, or with its spaces escaped), as itself or a path
    # in it: not a folder whose name only starts the same.
    for form in {str(src).casefold(), real, real.replace(" ", "\\ ")}:
        if re.search(re.escape(form) + r"""(?=$|[/\s'"\\;|&)<>])""", low):
            return True
    segs = _split_segments(command)
    if not segs:
        return False
    inside = lambda p: _inside(_resolve(p).casefold(), real)  # noqa: E731
    for start in (os.path.expanduser("~"), str(paths.root)):
        cwd = start
        for seg in segs:
            name, args = os.path.basename(seg[0]), seg[1:]
            if name in ("cd", "pushd"):
                targets = _shell_expand(args[0] if args else "~", cwd)
                if any(inside(t) for t in targets):
                    return True
                cwd = targets[0]
                continue
            if any(inside(p) for w in _positional(args, set()) for p in _shell_expand(w, cwd)):
                return True
    return False


def _runs_code(path: str) -> bool:
    home = os.path.realpath(os.path.expanduser("~"))
    if not path.startswith(home + "/"):
        return False
    rel = path[len(home) + 1:]
    return ("/" not in rel and rel.startswith(".")) or rel.startswith(_RUNS_CODE_DIRS)


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
        self._cwds: set[str] = set()                          # folders shell commands have moved to

    # ------------------------------------------------------------------ helpers
    def _path_denied(self, path: str | None) -> str | None:
        if path and is_private(self.paths, path):
            return "That file holds passwords or sign-in keys, so Chat Computer keeps it private."
        return None

    def _search_denied(self, root: str | None) -> str | None:
        """Searching file contents under a folder that holds Chat Computer's private files would read them too."""
        r = _resolve(root or "~")
        if _inside(r, "/proc") or holds_private(self.paths, r):
            return ("Searching inside the files there would also read Chat Computer's private sign-in files, so Chat Computer "
                    "blocks it. Search a more specific folder (like ~/Documents), or use search_files.")
        return None

    async def _ask(self, question: str) -> tuple[bool, str]:
        if self.confirm is None:
            return False, ""
        async with self._lock:  # one spoken question at a time
            verdict, said = await self.confirm(question)
        return verdict == "yes", said

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
        if said:
            return self._deny(f'The user did not approve. They said: "{said}". Do not retry this action; '
                              f"follow what they said or ask what they'd like instead.")
        return self._deny("The user did not answer, so this was not done. Mention it briefly and move on.")

    def _moved_to(self, command: str) -> None:
        starts = [os.path.expanduser("~"), str(self.paths.root), *sorted(self._cwds)]
        self._cwds |= _cd_targets(command, starts)
        if len(self._cwds) > 50:
            self._cwds = set(sorted(self._cwds)[-50:])

    # ------------------------------------------------------------------ the gate
    async def __call__(self, tool: str, inp: dict[str, Any], ctx: Any = None):
        # The tool call about to run (the model's run one at a time, each just after this): what it changes is recorded
        # as its, so Actions can take that back (Revert).
        self.journal.call = getattr(ctx, "tool_use_id", None) or None
        try:
            return await self._decide(tool, inp)
        except Exception as exc:  # never let a bug in the gate approve something by accident
            log.exception("permission gate error for %s", tool)
            return self._deny(f"Chat Computer couldn't check this action ({exc}).")

    async def _decide(self, tool: str, inp: dict[str, Any]):
        if tool in ("Read", "Glob", "Grep", "LS", "NotebookRead"):
            why = self._path_denied(inp.get("file_path") or inp.get("path") or inp.get("notebook_path"))
            if not why and tool == "Grep":
                why = self._search_denied(inp.get("path"))
            if why:
                self._note("deny", tool, inp)
                return self._deny(why)
            return _allow(inp)

        if tool in ("WebSearch", "WebFetch", "TodoWrite", "ToolSearch"):
            return _allow(inp)

        if tool in ("Write", "Edit", "MultiEdit", "NotebookEdit"):
            raw = inp.get("file_path") or inp.get("notebook_path") or ""
            if not raw:
                return self._deny("No file was named.")
            path = _resolve(raw)
            why = self._path_denied(path)
            if why or is_program(path):
                self._note("deny", tool, inp)
                return self._deny(why or PROGRAM_DENIED)
            if _runs_code(path):
                ok, said = await self._ask(f"Can I change {os.path.basename(path)}? Your computer runs what's in it.")
                self._note("allow" if ok else "deny", tool, inp)
                if not ok:
                    return self._user_said_no(said)
            existed = os.path.isfile(path)
            backup = await asyncio.to_thread(self.journal.backup, path) if existed else None
            self.journal.record_overwrite(path, backup, "changed" if existed else "created")
            return _allow(inp)

        if tool == "Bash":
            cmd = str(inp.get("command", ""))
            verdict, why = classify_bash(cmd)
            if verdict != "deny" and _reaches_private(self.paths, cmd, self._cwds):
                verdict, why = "deny", "That would read Chat Computer's private sign-in files, so Chat Computer blocks it."
            if verdict == "ask" and _changes_update_source(self.paths, cmd):
                verdict, why = "deny", PROGRAM_DENIED
            if verdict == "deny":
                self._note("deny", tool, inp)
                return self._deny(why)
            if verdict == "allow":
                self._moved_to(cmd)
                return _allow(inp)
            desc = (inp.get("description") or "").strip().rstrip(".")
            question = (f"Can I run a command to {desc[0].lower() + desc[1:]}?" if desc and len(desc) > 3
                        else "Can I run this command: " + _short(cmd) + "?")
            ok, said = await self._ask(question)
            self._note("allow" if ok else "deny", tool, inp)
            if ok:
                self._moved_to(cmd)
                self.journal.record("command", desc or _short(cmd), command=cmd)
                return _allow(inp)
            return self._user_said_no(said)

        if tool.startswith("mcp__chatcomputer__"):
            name = tool[len("mcp__chatcomputer__"):]
            policy = self.tool_policies.get(name)
            question = policy(inp) if policy else None
            if question and question.startswith("DENY:"):
                self._note("deny", tool, inp)
                return self._deny(question[5:].strip())
            if not question:
                return _allow(inp)
            ok, said = await self._ask(question)
            self._note("allow" if ok else "deny", tool, inp)
            return _allow(inp) if ok else self._user_said_no(said)

        # Any other tool (e.g. one added by a future engine version): ask.
        ok, said = await self._ask(f"Can I use {tool}?")
        return _allow(inp) if ok else self._user_said_no(said)


def _allow(inp: dict[str, Any]) -> PermissionResultAllow:
    return PermissionResultAllow(updated_input=inp)


def _short(cmd: str, n: int = 80) -> str:
    cmd = " ".join(cmd.split())
    return cmd if len(cmd) <= n else cmd[: n - 1] + "…"
