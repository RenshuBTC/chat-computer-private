#!/bin/bash
# Builds ClaudeOS.zip: the folder a person downloads and extracts. On Windows they run "ClaudeOS Setup.cmd"
# from it; on a Mac, mac/setup.sh in Terminal (or "ClaudeOS Setup.command"). Usage: scripts/make-package.sh OUT_DIR
set -euo pipefail
REPO="$(cd "$(dirname "$0")/.." && pwd)"
OUT="${1:?usage: make-package.sh OUT_DIR}"
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
DEST="$STAGE/ClaudeOS"
mkdir -p "$DEST"
# os/ too: "ClaudeOS Own System" in windows/ signs and starts os/release/claudeos.efi (its build folders stay out).
( cd "$REPO" && tar --exclude='.venv' --exclude='.pytest_cache' --exclude='__pycache__' --exclude='*.pyc' \
    --exclude='.git' --exclude='os/target' --exclude='os/out' -cf - ARCHITECTURE.md README.md pyproject.toml \
    requirements-linux.txt claudeos linux windows mac os tests scripts ) | tar -xf - -C "$DEST"
# The things to double-click sit at the top of the folder: one for Windows, one for a Mac.
cp "$REPO/windows/ClaudeOS Setup.cmd" "$DEST/ClaudeOS Setup.cmd"
cp -p "$REPO/ClaudeOS Setup.command" "$DEST/ClaudeOS Setup.command"
chmod 755 "$DEST/ClaudeOS Setup.command" "$DEST/mac/setup.sh" "$DEST/mac/drive-installer.exp"
mkdir -p "$OUT"
rm -f "$OUT/ClaudeOS.zip"
( cd "$STAGE" && zip -q -r -X "$OUT/ClaudeOS.zip" ClaudeOS )
echo "$OUT/ClaudeOS.zip"
