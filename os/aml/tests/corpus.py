#!/usr/bin/env python3
"""Checks the interpreter against ACPICA's acpiexec on real PCs' ACPI tables: the linuxhw/ACPI collection (acpidump's
output from thousands of PCs, https://github.com/linuxhw/ACPI). For each PC picked, its DSDT and SSDTs are loaded into
both, and every device's standard objects (_HID, _STA, _CRS, _PRT, a battery's _BIF and _BST...) evaluated in turn in
both, in one session each; the results are compared. Operation regions read as zeros until written, in both.

    python3 tests/corpus.py REPO [--count 40] [--kind Notebook|Desktop] [--seed 1] [--only "ThinkPad T480s 20L7S00600"]

REPO is a clone of linuxhw/ACPI (a partial one is enough: the files picked are fetched). Needs acpica-tools. Ours is
set up as acpiexec sets itself up (examples/load.rs): the same _REG, _INI, _STA and _HID evaluations in the same order,
since methods have side effects the later results show. A PC with operation regions in spaces acpiexec simulates in a
way of its own (serial buses, GPIO, IPMI...) has its differences counted apart."""
from __future__ import annotations

import argparse
import random
import re
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent


def parse(lines: list[str]) -> tuple[str, int]:
    line = lines[0].strip()
    if line.startswith("[Integer] = "):
        return f"I:{int(line.split()[2], 16):X}", 1
    if line.startswith("[String]"):
        a, b = line.find('"'), line.rfind('"')
        return f'"{line[a + 1:b]}"', 1
    if line.startswith("[Buffer] Length "):
        count = int(line.split()[2], 16)
        out, used = [], 0
        for l in lines:
            if len(out) >= count and used:
                break
            used += 1
            if ": " not in l:
                continue
            hexpart = l.split(": ", 1)[1].split("//")[0]
            out += [h for h in hexpart.split() if len(h) == 2]
            if count == 0:
                break
        return f"B[{' '.join(out[:count])}]", max(used, 1)
    if line.startswith("[Package] Contains "):
        count = int(line.split()[2])
        at, items = 1, []
        for _ in range(count):
            if at >= len(lines):
                break
            item, used = parse(lines[at:])
            items.append(item)
            at += used
        return f"P[{', '.join(items)}]", at
    if line.startswith("[Null Object]"):
        return "U", 1
    if line.startswith("[Object Reference]") or "Reference" in line:
        m = re.search(r"\\[\w.]+", line)
        return (f"R:{m.group(0)}" if m else "R"), 1
    return f"?{line}", 1


def acpiexec(tables: list[Path], paths: list[str]) -> dict[str, str]:
    commands = "".join(f"evaluate {p}\n" for p in paths) + "quit\n"
    out = subprocess.run(["acpiexec", "-dr", *map(str, tables)], input=commands, capture_output=True, text=True,
                         errors="replace", timeout=600).stdout
    lines = out.splitlines()
    found = {}
    for i, l in enumerate(lines):
        if l.startswith("No object was returned from evaluation of "):
            found[l.split()[-1]] = "U"
        elif l.startswith("Evaluation of "):
            path = l.split()[2]
            if "failed with status AE_BUFFER_OVERFLOW" in l:
                found[path] = "too big"  # (for acpiexec's debugger to show: not compared)
            elif "failed with status" in l:
                found[path] = "error"
            elif "returned object" in l:
                found[path] = parse(lines[i + 1:])[0]
    return found


def ours(tables: list[Path]) -> tuple[list[tuple[str, str]], list[str]]:
    exe = subprocess.run(["cargo", "build", "--release", "--example", "load", "--message-format", "short"], cwd=HERE,
                         capture_output=True, text=True)
    if exe.returncode:
        sys.exit(exe.stderr)
    target = Path(subprocess.run(["cargo", "metadata", "--format-version", "1", "--no-deps"], cwd=HERE, capture_output=True,
                                 text=True).stdout.split('"target_directory":"')[1].split('"')[0])
    out = subprocess.run([str(target / "release/examples/load"), "--standard", *map(str, tables)], capture_output=True,
                         text=True, errors="replace", timeout=600).stdout
    results, problems, unlike = [], [], []
    for l in out.splitlines():
        if l.startswith("= "):
            path, value = l[2:].split(" ", 1)
            results.append((path, value))
        elif l.startswith("problem ") or l.startswith("load "):
            problems.append(l)
        elif l.startswith("unlike "):
            unlike = l.split()[1:]
    return results, problems, unlike


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("repo")
    ap.add_argument("--count", type=int, default=40)
    ap.add_argument("--kind", default="Notebook")
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--only", action="append", help="just the PCs whose folder names have this in them")
    args = ap.parse_args()
    repo = Path(args.repo)
    files = subprocess.run(["git", "ls-tree", "-r", "--name-only", "HEAD"], cwd=repo, capture_output=True, text=True).stdout.splitlines()
    dumps = sorted(f for f in files if f.endswith(".bin") and f.startswith(args.kind + "/"))
    random.Random(args.seed).shuffle(dumps)
    picked = [d for d in dumps if any(f"/{o}/" in d for o in args.only)] if args.only else dumps[:args.count]
    subprocess.run(["git", "checkout", "HEAD", "--", *picked], cwd=repo, capture_output=True)
    total = same = expected = 0
    worst: list[str] = []
    for dump in picked:
        with tempfile.TemporaryDirectory() as tmp:
            subprocess.run(["acpixtract", "-a", str(repo / dump)], cwd=tmp, capture_output=True)
            dsdt = Path(tmp) / "dsdt.dat"
            if not dsdt.exists():
                print(f"{dump}: no DSDT")
                continue
            tables = [dsdt] + sorted(Path(tmp).glob("ssdt*.dat"), key=lambda p: int(re.sub(r"\D", "", p.stem) or 0))
            mine, problems, unlike = ours(tables)
            theirs = acpiexec(tables, [p for p, _ in mine])
            same_as = lambda v, t: (v == t or t == "too big" or (v.startswith("error") and t == "error")
                                    or re.sub(r"R:[^,\]]*", "R", v) == t)
            differ = [(p, v, theirs.get(p, "(none)")) for p, v in mine if not same_as(v, theirs.get(p, "(none)"))]
            total += len(mine)
            same += len(mine) - len(differ)
            name = dump.rsplit("/", 2)[-2]
            # A PC with regions in spaces acpiexec simulates its own way (serial buses, GPIO...): its differences may
            # come from that, so they're counted apart.
            note = f" (regions in spaces {', '.join(unlike)}, which acpiexec simulates its own way)" if unlike else ""
            if unlike:
                expected += len(differ)
            print(f"{name}: {len(mine) - len(differ)}/{len(mine)} the same, {len(problems)} load problems{note}")
            for p, v, t in differ[:6]:
                worst.append(f"  {name} {p}: ours {v[:150]}, acpiexec's {t[:150]}")
                print(worst[-1])
    print(f"all: {same}/{total} the same; {expected} of the differences on PCs with regions acpiexec simulates its own way")
    return 0 if same + expected == total else 1


if __name__ == "__main__":
    sys.exit(main())
