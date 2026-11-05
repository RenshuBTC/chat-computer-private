#!/bin/bash
# Chat Computer Setup for Mac. Paste this into Terminal (in Applications, then Utilities) and press Return:
#     curl -fsSL https://chatcomputer.ai/get/mac.sh | bash
# It downloads the app (one disk image, @DMG@), checks its SHA-256 against the one written here, puts Chat Computer
# in Applications (the Mac's, or your own ~/Applications where that isn't yours to change: no administrator password
# either way) and opens it. The app then asks how you want Chat Computer: as an app, or as its own system next to
# macOS (on a Mac with M1, M2 or M3).
# A file curl downloads isn't marked as coming from the internet, so macOS opens the app without asking you to choose
# Open Anyway in System Settings (it isn't signed with a paid Apple certificate yet).
# Made by scripts/build_apps.py from the disk image beside it on chatcomputer.ai/get: don't change it by hand.
set -euo pipefail
url='https://chatcomputer.ai/get/@DMG@'
sha256='@SHA256@'
app='Chat Computer.app'
[ "$(uname -s)" = Darwin ] || { echo 'This is Chat Computer Setup for a Mac.' >&2; exit 1; }
work="$(mktemp -d "${TMPDIR:-/tmp}/chat-computer.XXXXXX")"
disk="$work/disk"
cleanup() { hdiutil detach "$disk" -quiet >/dev/null 2>&1 || true; rm -rf "$work"; }
trap cleanup EXIT
echo 'Downloading Chat Computer...'
curl -fL --progress-bar "$url" -o "$work/@DMG@"
echo "$sha256  $work/@DMG@" | shasum -a 256 -c - >/dev/null 2>&1 || {
  echo "The download isn't the one Chat Computer published (its SHA-256 doesn't match), so nothing was installed." >&2
  echo 'Try again in a while.' >&2
  exit 1
}
mkdir "$disk"
hdiutil attach "$work/@DMG@" -nobrowse -readonly -noautoopen -mountpoint "$disk" -quiet
dest=/Applications
if [ ! -w "$dest" ]; then dest="$HOME/Applications"; mkdir -p "$dest"; fi
rm -rf "$dest/$app.new"
ditto "$disk/$app" "$dest/$app.new"
rm -rf "$dest/$app"
mv "$dest/$app.new" "$dest/$app"
echo "Chat Computer is in $dest."
if pgrep -f "/$app/Contents/MacOS/" >/dev/null 2>&1; then
  echo 'It was open: quit it (Command-Q) and open it again for the new one.'
fi
echo 'Opening it: choose how you want Chat Computer.'
open "$dest/$app"
