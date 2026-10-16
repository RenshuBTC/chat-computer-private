#!/bin/bash
# Builds mac/ClaudeOS.app, the app ClaudeOS Setup copies to ~/Applications on a Mac:
#   1. the main program from launcher.c, for Apple silicon and for Intel, with Zig's C compiler (it can make
#      Mac programs on any computer: pip install ziglang), joined into one universal program;
#   2. the app folder: Info.plist (mac/app), the program, the icon (mac/ClaudeOS.icns);
#   3. an ad hoc signature over all of it, made with rcodesign (apple-codesign), so nothing has to be signed
#      on the Mac (that would need Apple's developer tools) and the app is the same everywhere.
# Usage: mac/launcher/build.sh PYTHON_WITH_ZIGLANG RCODESIGN
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
MAC="$(cd "$HERE/.." && pwd)"
PY="${1:?usage: build.sh PYTHON_WITH_ZIGLANG RCODESIGN}"
RCODESIGN="${2:?usage: build.sh PYTHON_WITH_ZIGLANG RCODESIGN}"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

for arch in arm64 x86_64; do
  case "$arch" in arm64) target=aarch64-macos.11.0 ;; x86_64) target=x86_64-macos.11.0 ;; esac
  (cd "$WORK" && "$PY" -m ziglang cc -target "$target" -O2 -s -Wall -Wextra -Werror -o "ClaudeOS-$arch" "$HERE/launcher.c")
done
"$PY" "$HERE/universal.py" "$WORK/ClaudeOS" "$WORK/ClaudeOS-arm64" "$WORK/ClaudeOS-x86_64"

APP="$MAC/ClaudeOS.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$MAC/app/Info.plist" "$APP/Contents/Info.plist"
cp "$WORK/ClaudeOS" "$APP/Contents/MacOS/ClaudeOS"
chmod 755 "$APP/Contents/MacOS/ClaudeOS"
cp "$MAC/ClaudeOS.icns" "$APP/Contents/Resources/ClaudeOS.icns"
"$RCODESIGN" sign "$APP" > "$WORK/sign.log" 2>&1 || { cat "$WORK/sign.log"; exit 1; }

{
  echo "zig $("$PY" -c 'import importlib.metadata as m; print(m.version("ziglang"))')"
  "$RCODESIGN" --version
  cd "$MAC"
  if command -v sha256sum >/dev/null 2>&1; then sum=(sha256sum); else sum=(shasum -a 256); fi
  "${sum[@]}" ClaudeOS.app/Contents/MacOS/ClaudeOS ClaudeOS.app/Contents/Info.plist \
    ClaudeOS.app/Contents/_CodeSignature/CodeResources
} > "$HERE/BUILT-WITH.txt"
cat "$HERE/BUILT-WITH.txt"
