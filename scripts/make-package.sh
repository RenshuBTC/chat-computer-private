#!/bin/bash
# Builds ChatComputer.zip: the folder a person downloads and extracts. On Windows they run "Chat Computer Setup.cmd"
# from it; on a Mac, mac/setup.sh in Terminal (or "Chat Computer Setup.command"). Usage: scripts/make-package.sh OUT_DIR
set -euo pipefail
REPO="$(cd "$(dirname "$0")/.." && pwd)"
OUT="${1:?usage: make-package.sh OUT_DIR}"
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
DEST="$STAGE/ChatComputer"
mkdir -p "$DEST"
# os/ too: "Chat Computer Own System" in windows/ signs and starts os/release/chatcomputer.efi (its build folders stay out).
( cd "$REPO" && tar --exclude='.venv' --exclude='.pytest_cache' --exclude='__pycache__' --exclude='*.pyc' \
    --exclude='.git' --exclude='os/target' --exclude='os/out' -cf - ARCHITECTURE.md README.md pyproject.toml \
    requirements-linux.txt chatcomputer linux windows mac os tests scripts ) | tar -xf - -C "$DEST"
# The things to double-click sit at the top of the folder: one for Windows, one for a Mac.
cp "$REPO/windows/Chat Computer Setup.cmd" "$DEST/Chat Computer Setup.cmd"
cp -p "$REPO/Chat Computer Setup.command" "$DEST/Chat Computer Setup.command"
chmod 755 "$DEST/Chat Computer Setup.command" "$DEST/mac/setup.sh" "$DEST/mac/drive-installer.exp"
mkdir -p "$OUT"
rm -f "$OUT/ChatComputer.zip"
( cd "$STAGE" && zip -q -r -X "$OUT/ChatComputer.zip" ChatComputer )
echo "$OUT/ChatComputer.zip"
