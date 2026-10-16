#!/bin/bash
# ClaudeOS Setup for Mac. Run it in Terminal:
#     bash ~/Downloads/ClaudeOS/mac/setup.sh
# (or double-click "ClaudeOS Setup.command" next to this folder).
#
# ClaudeOS on a Mac is an app that fills the screen; macOS, your apps and your files stay as they are.
# Installing it:
#   1. checks this Mac (Apple silicon with macOS 14 or newer, or an Intel Mac with macOS 13 or newer),
#   2. gets uv (Astral's Python installer, from PyPI, checked against a fixed SHA-256), and with it a
#      private Python and ClaudeOS's packages, all inside ~/ClaudeOS,
#   3. copies ClaudeOS to ~/ClaudeOS/app,
#   4. signs in to Claude with your subscription (the official Claude Code engine that comes with
#      Anthropic's Agent SDK), and, for typing from the Claude app on your phone, a full sign-in,
#   5. downloads the voice and speech recognition (about 600 MB),
#   6. puts the ClaudeOS app in ~/Applications (built and signed ahead of time, mac/ClaudeOS.app; its small
#      main program runs ClaudeOS's Python inside the app, so macOS asks for the microphone and Bluetooth for
#      "ClaudeOS"), and opens it when you log in if you want,
#   7. starts ClaudeOS.
# It can also sign in again, change whether ClaudeOS opens at login, and remove ClaudeOS completely.
# Written for the bash that comes with macOS (3.2).
set -u

ROOT="${CLAUDEOS_MAC_ROOT:-$HOME}"           # tests point this at a pretend home folder
CO="$ROOT/ClaudeOS"
APP="$ROOT/Applications/ClaudeOS.app"
AGENT="$ROOT/Library/LaunchAgents/com.claudeos.start.plist"
HERE="$(cd "$(dirname "$0")" && pwd)"
SRC="$(cd "$HERE/.." && pwd)"                 # the folder you downloaded (it has claudeos/ and mac/)
UV_VERSION="0.12.20"
UV_ARM_URL="https://files.pythonhosted.org/packages/95/02/857f98f06d91a9c932e04d911367623134dfc941968d7d039678287172e9/uv-0.12.20-py3-none-macosx_11_0_arm64.whl"
UV_ARM_SHA="08ca4588bac6a4bb8972236c0958aaa0ebea7b2045289edbf85f9a21f44db979"
UV_INTEL_URL="https://files.pythonhosted.org/packages/a7/50/4e0552f3d40c05f164e1e211ed4537defd5d9f4233f08f84f973ac8390b4/uv-0.12.20-py3-none-macosx_10_12_x86_64.whl"
UV_INTEL_SHA="108e4bce126b0baec03dd035b656fe4ce614d336e182eff02abec7df5212a86c"
LOG="$CO/logs/setup.log"

# ------------------------------------------------------------------ talking to the person
# The log starts once installing starts, so looking at the menu leaves nothing behind.
# Anything shaped like a Claude sign-in is blanked out before it is written.
log() {
  if [ -d "$CO/logs" ]; then
    printf '%s  %s\n' "$(date '+%H:%M:%S')" "$(printf '%s' "$*" | sed -E 's/sk-ant-[A-Za-z0-9_-]+/sk-ant-(hidden)/g')" \
      >> "$LOG" 2>/dev/null
  fi
  return 0
}
say() { printf '%s\n' "$*"; log "$*"; }
step() { printf '\n\033[1;36m%s\033[0m\n' "$*"; log "== $*"; }
good() { printf '\033[32m%s\033[0m\n' "$*"; log "OK $*"; }
warn() { printf '\033[33m%s\033[0m\n' "$*"; log "WARNING $*"; }
stop() { printf '\n\033[33mSetup stopped: %s\033[0m\n' "$*"; log "STOPPED $*"; exit 1; }

ask_yes() {   # ask_yes QUESTION DEFAULT(y|n)
  local q="$1" def="$2" a hint="[y/N]"
  [ "$def" = "y" ] && hint="[Y/n]"
  while true; do
    printf '%s %s ' "$q" "$hint"
    read -r a || a=""
    [ -z "$a" ] && a="$def"
    case "$a" in
      y|Y|yes|Yes|YES) log "Q: $q -> y"; return 0 ;;
      n|N|no|No|NO) log "Q: $q -> n"; return 1 ;;
    esac
    # Only real answers are logged: anything else could be part of something secret typed in the wrong place.
    log "Q: $q -> (not y or n)"
    printf 'Please type y for yes or n for no.\n'
  done
}

ask_choice() {   # ask_choice QUESTION OPTION... ; sets CHOICE to the number picked (1-based)
  local q="$1" i=1 a
  shift
  printf '\n%s\n' "$q"
  for o in "$@"; do printf '  %d. %s\n' "$i" "$o"; i=$((i + 1)); done
  while true; do
    printf 'Type a number (Enter = 1) '
    read -r a || a=""
    [ -z "$a" ] && a=1
    case "$a" in
      *[!0-9]*) ;;
      *) if [ "$a" -ge 1 ] && [ "$a" -le "$#" ]; then CHOICE="$a"; log "Q: $q -> $a"; return 0; fi ;;
    esac
    log "Q: $q -> (not a number on the list)"
    printf 'Please type a number from 1 to %s.\n' "$#"
  done
}

# Reads what's pasted, hidden, however many lines it has, into PASTED, up to the Enter that follows it.
# Claude's sign-in code is longer than Terminal's usual width and Claude Code wraps it with real line breaks,
# so a copy of it can hold two or more lines. Terminal marks where a paste starts and ends when asked to
# (bracketed paste), so line breaks inside the paste don't end it, and the Enter pressed after it does.
# A terminal without those marks: a line break followed at once by more text is still part of the paste.
STTY_SAVED=""
restore_terminal() {
  if [ -n "$STTY_SAVED" ]; then
    printf '\033[?2004l'
    stty "$STTY_SAVED" 2>/dev/null
    STTY_SAVED=""
  fi
}
trap 'restore_terminal' EXIT
trap 'restore_terminal; printf "\n"; exit 130' INT TERM
read_chunk() {   # one read of whatever has arrived (up to 4 KB), kept exactly, newlines included
  CHUNK="$(dd bs=4096 count=1 2>/dev/null; printf x)"
  CHUNK="${CHUNK%x}"
}
read_paste() {
  local all="" rest start end
  start=$'\033[200~'
  end=$'\033[201~'
  if ! STTY_SAVED="$(stty -g 2>/dev/null)"; then    # not a terminal: one line
    STTY_SAVED=""
    read -r all || all=""
    PASTED="$all"
    return 0
  fi
  stty -echo -icanon min 1 time 0
  printf '\033[?2004h'
  while :; do
    read_chunk
    [ -z "$CHUNK" ] && break                        # the input has ended
    all="$all$CHUNK"
    if [ "${all#*"$start"}" != "$all" ]; then       # something was pasted with marks...
      case "${all##*"$start"}" in *"$end"*) ;; *) continue ;; esac    # ...and it hasn't ended yet
    fi
    rest="${all##*"$end"}"                          # what was typed after the paste
    case "$rest" in *$'\n'*|*$'\r'*) ;; *) continue ;; esac            # no Enter yet
    stty min 0 time 5                               # an Enter: anything still arriving within half a second
    read_chunk                                      # belongs to an unmarked paste
    stty min 1 time 0
    [ -z "$CHUNK" ] && break
    all="$all$CHUNK"
  done
  restore_terminal
  PASTED="$all"
}

# ------------------------------------------------------------------ this Mac
check_mac() {
  step "Checking this Mac"
  ARCH="$(uname -m)"
  # A Terminal running under Rosetta says x86_64 on Apple silicon too; ask the hardware.
  if [ "$(sysctl -n hw.optional.arm64 2>/dev/null)" = 1 ]; then ARCH=arm64; fi
  local ver major
  ver="$(sw_vers -productVersion 2>/dev/null)"
  major="${ver%%.*}"
  case "$major" in ''|*[!0-9]*) stop "This doesn't look like a Mac (no macOS version found)." ;; esac
  case "$ARCH" in
    arm64)
      [ "$major" -ge 14 ] || stop "ClaudeOS needs macOS 14 (Sonoma) or newer on this Mac; it has macOS $ver. Update it in System Settings, then run setup again." ;;
    x86_64)
      [ "$major" -ge 13 ] || stop "ClaudeOS needs macOS 13 (Ventura) or newer on this Mac; it has macOS $ver." ;;
    *)
      stop "This Mac's processor ($ARCH) isn't one ClaudeOS supports." ;;
  esac
  say "macOS $ver on $( [ "$ARCH" = arm64 ] && echo "Apple silicon" || echo "an Intel processor" )."
  local free_kb
  free_kb="$(df -k "$ROOT" 2>/dev/null | awk 'NR==2 {print $4}')"
  case "$free_kb" in ''|*[!0-9]*) free_kb=0 ;; esac
  if [ "$free_kb" -lt 5000000 ]; then
    stop "ClaudeOS needs about 5 GB of free space; this Mac has $((free_kb / 1000000)) GB. Free some space, then run setup again."
  fi
  if ! curl -sSfI --max-time 20 https://pypi.org/simple/ -o /dev/null 2>/dev/null; then
    stop "This Mac can't reach the internet (pypi.org). Connect to Wi-Fi, then run setup again."
  fi
}

# ------------------------------------------------------------------ the program
get_uv() {
  UV="${CLAUDEOS_UV:-}"                       # tests bring their own
  if [ -n "$UV" ]; then return 0; fi
  local url sha whl got
  if [ "$ARCH" = arm64 ]; then url="$UV_ARM_URL"; sha="$UV_ARM_SHA"; else url="$UV_INTEL_URL"; sha="$UV_INTEL_SHA"; fi
  mkdir -p "$CO/.setup"
  whl="$CO/.setup/uv-$UV_VERSION.whl"
  UV="$CO/.setup/uv-$UV_VERSION.data/scripts/uv"
  if [ -x "$UV" ]; then return 0; fi
  say "Downloading uv, which installs ClaudeOS's own Python (about 20 MB)..."
  curl -sSfL --retry 3 -o "$whl" "$url" || stop "The download of uv failed. Check the internet connection and try again."
  got="$(shasum -a 256 "$whl" | awk '{print $1}')"
  if [ "$got" != "$sha" ]; then
    rm -f "$whl"
    stop "The uv download didn't match its known fingerprint (got $got). Nothing was installed."
  fi
  (cd "$CO/.setup" && unzip -o -q "$whl" "uv-$UV_VERSION.data/scripts/uv") || stop "Couldn't unpack uv."
  chmod 755 "$UV"
}

copy_app() {
  say "Copying ClaudeOS..."
  rm -rf "$CO/app.new"
  mkdir -p "$CO/app.new"
  cp -R "$SRC/claudeos" "$SRC/mac" "$CO/app.new/" || stop "Couldn't copy ClaudeOS from $SRC."
  for f in README.md ARCHITECTURE.md pyproject.toml; do
    [ -f "$SRC/$f" ] && cp "$SRC/$f" "$CO/app.new/"
  done
  find "$CO/app.new" -name '__pycache__' -type d -prune -exec rm -rf {} + 2>/dev/null
  xattr -dr com.apple.quarantine "$CO/app.new" 2>/dev/null || true
  rm -rf "$CO/app"
  mv "$CO/app.new" "$CO/app"
}

make_runtime() {
  say "Installing ClaudeOS's Python and packages (about 1 GB; a few minutes)..."
  export UV_PYTHON_INSTALL_DIR="$CO/python" UV_CACHE_DIR="$CO/.setup/cache"
  # After downloading Python, uv renames its library with Apple's install_name_tool. On a Mac without the
  # developer tools that command only asks to install them (a window would pop up), and ClaudeOS doesn't
  # need the rename, so uv gets one that does nothing.
  mkdir -p "$CO/.setup/bin"
  printf '#!/bin/sh\nexit 0\n' > "$CO/.setup/bin/install_name_tool"
  chmod 755 "$CO/.setup/bin/install_name_tool"
  if [ ! -x "$CO/runtime/bin/python" ]; then
    PATH="$CO/.setup/bin:$PATH" "$UV" venv --python 3.12 --python-preference only-managed "$CO/runtime" >> "$LOG" 2>&1 \
      || stop "Couldn't make ClaudeOS's Python. Details are in $LOG."
  fi
  # Ready-made packages only (--only-binary): nothing is compiled on this Mac, so no developer tools are needed.
  "$UV" pip install --python "$CO/runtime/bin/python" --only-binary :all: -r "$CO/app/mac/requirements-mac.txt" >> "$LOG" 2>&1 \
    || stop "Couldn't install ClaudeOS's packages. Check the internet connection and try again (details: $LOG)."
  # ClaudeOS itself isn't a package: a .pth file puts ~/ClaudeOS/app on the import path.
  local site
  site="$("$CO/runtime/bin/python" -c 'import sysconfig; print(sysconfig.get_paths()["purelib"])')" \
    || stop "ClaudeOS's Python doesn't start."
  printf '%s\n' "$CO/app" > "$site/claudeos-app.pth"
  (cd / && CLAUDEOS_HOME="$CO" "$CO/runtime/bin/python" -c 'import claudeos.config') \
    || stop "ClaudeOS's Python can't load ClaudeOS. Details are in $LOG."
  rm -rf "$CO/.setup/cache"
  good "ClaudeOS is installed in $CO."
}

write_settings() {
  [ -f "$CO/settings.json" ] && return 0
  local name zone
  name="$(id -F 2>/dev/null || true)"
  zone="$(readlink /etc/localtime 2>/dev/null | sed -n 's|.*zoneinfo/||p')"
  CLAUDEOS_NAME="$name" CLAUDEOS_ZONE="$zone" "$CO/runtime/bin/python" -c '
import json, os, sys
s = {"user_name": os.environ["CLAUDEOS_NAME"], "timezone": os.environ["CLAUDEOS_ZONE"], "locale": "en_US.UTF-8"}
open(sys.argv[1], "w", encoding="utf-8").write(json.dumps(s, indent=2, ensure_ascii=False) + "\n")
' "$CO/settings.json"
}

# ------------------------------------------------------------------ signing in to Claude
engine() {   # the Claude Code engine that comes with the Agent SDK
  if [ -n "${CLAUDEOS_ENGINE_CLI:-}" ]; then printf '%s\n' "$CLAUDEOS_ENGINE_CLI"; return; fi
  "$CO/runtime/bin/python" -c 'from claudeos.config import engine_cli; print(engine_cli() or "")' 2>/dev/null
}

sign_in() {
  step "Sign in to Claude"
  local cli tmp tok raw try=1
  cli="$(engine)"
  if [ -z "$cli" ] || [ ! -x "$cli" ]; then stop "The Claude engine is missing from ClaudeOS's Python. Run setup again."; fi
  say "Your browser opens to sign in to Claude. Use the account with your Claude subscription."
  say "When it finishes, this window shows a long code starting with sk-ant-. It may run over two lines:"
  say "copy all of it."
  tmp="$(mktemp -d)"
  CLAUDE_CONFIG_DIR="$tmp" "$cli" setup-token
  rm -rf "$tmp"
  while [ $try -le 3 ]; do
    printf 'Paste the code here (it stays hidden), then press Enter: '
    read_paste
    raw="$PASTED"
    printf '\n'
    tok="$(printf '%s' "$raw" | tr -d ' \t\r\n"'"'" | grep -oE 'sk-ant-(oat01|api03)-[A-Za-z0-9_-]{20,}' | head -n 1)"
    if [ -n "$tok" ]; then
      mkdir -p "$CO/secrets"
      chmod 700 "$CO/secrets"
      (umask 077 && printf '%s\n' "$tok" > "$CO/secrets/claude_token")
      good "Got it. Claude is signed in for ClaudeOS."
      break
    fi
    warn "That doesn't look like the code. It starts with sk-ant- and is about 100 characters long."
    try=$((try + 1))
  done
  [ -n "${tok:-}" ] || stop "No valid Claude sign-in code was entered."
  say ""
  say "Besides talking, you can type to ClaudeOS in the Claude app on your phone: the phone is its keyboard."
  if ask_yes "Set up typing from your phone? (your browser opens once more)" y; then
    mkdir -p "$CO/.engine"
    chmod 700 "$CO/.engine"
    CLAUDE_CONFIG_DIR="$CO/.engine" "$cli" auth login --claudeai
    if env -u CLAUDE_CODE_OAUTH_TOKEN -u ANTHROPIC_API_KEY CLAUDE_CONFIG_DIR="$CO/.engine" "$cli" auth status --json 2>/dev/null \
        | grep -q '"authMethod": *"claude.ai"'; then
      good "Done. In the Claude app on your phone, tap Code, then ClaudeOS."
    else
      warn "That sign-in didn't finish. ClaudeOS works without it; run setup again and choose to sign in again to add it."
    fi
  fi
}

# ------------------------------------------------------------------ the app and starting at login
python_library() {   # the shared Python library the app's main program loads
  "$CO/runtime/bin/python" -c '
import os, sys, sysconfig
name = sysconfig.get_config_var("INSTSONAME") or sysconfig.get_config_var("LDLIBRARY") or ""
for d in (os.path.join(sys.base_prefix, "lib"), sysconfig.get_config_var("LIBDIR") or ""):
    if name and os.path.isfile(os.path.join(d, name)):
        print(os.path.join(d, name))
        break
' 2>> "$LOG"
}

make_app() {
  # The app is built and signed ahead of time (mac/launcher/build.sh): the same for every Mac, so nothing is
  # signed here (that would need Apple's developer tools). Where ClaudeOS's Python is goes next to it.
  local lib
  [ -f "$CO/app/mac/ClaudeOS.app/Contents/MacOS/ClaudeOS" ] \
    || stop "The ClaudeOS app is missing from the download (mac/ClaudeOS.app). Download ClaudeOS again."
  lib="$(python_library)"
  [ -n "$lib" ] || stop "ClaudeOS's Python has no shared library for the app to use. Details are in $LOG."
  printf 'python=%s\nlibpython=%s\n' "$CO/runtime/bin/python" "$lib" > "$CO/launch.conf"
  rm -rf "$APP"
  mkdir -p "$ROOT/Applications"
  cp -Rp "$CO/app/mac/ClaudeOS.app" "$APP" || stop "Couldn't put the ClaudeOS app in $ROOT/Applications."
  chmod 755 "$APP/Contents/MacOS/ClaudeOS"
  xattr -cr "$APP" 2>/dev/null || true
}

stop_running() {   # quits ClaudeOS if it's open, so its files can be replaced or removed
  pkill -f "ClaudeOS.app/Contents/MacOS/ClaudeOS" >/dev/null 2>&1 || true
  pkill -f "claudeos run" >/dev/null 2>&1 || true
}

open_at_login() {   # open_at_login on|off
  if [ "$1" = on ]; then
    mkdir -p "$(dirname "$AGENT")"
    cat > "$AGENT" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>com.claudeos.start</string>
  <key>ProgramArguments</key><array><string>/usr/bin/open</string><string>-a</string><string>$APP</string></array>
  <key>RunAtLoad</key><true/>
  <key>ProcessType</key><string>Interactive</string>
</dict>
</plist>
PLIST
  else
    launchctl bootout "gui/$(id -u)/com.claudeos.start" >/dev/null 2>&1 || true
    rm -f "$AGENT"
  fi
}

# ------------------------------------------------------------------ the four things setup does
install() {
  check_mac
  step "What happens"
  say "ClaudeOS goes into $CO (about 3 GB, with the voice), with an app in $ROOT/Applications."
  say "Nothing else on this Mac changes. You can switch back to macOS any time, and remove ClaudeOS with this setup."
  ask_yes "Go ahead?" y || { say "Nothing was changed."; exit 0; }
  mkdir -p "$CO/logs"
  log "Setup on macOS $(sw_vers -productVersion 2>/dev/null) ($ARCH), from $SRC"
  step "Installing"
  stop_running
  get_uv
  copy_app
  make_runtime
  write_settings
  if [ -s "$CO/secrets/claude_token" ] && ! ask_yes "ClaudeOS is already signed in to Claude. Sign in again?" n; then
    :
  else
    sign_in
  fi
  if [ -z "${CLAUDEOS_SKIP_MODELS:-}" ]; then
    step "The voice"
    say "Downloading the voice and speech recognition (about 600 MB)..."
    CLAUDEOS_HOME="$CO" "$CO/runtime/bin/python" -m claudeos models --download >> "$LOG" 2>&1 \
      || warn "The voice didn't download now; ClaudeOS downloads it when it starts."
  fi
  make_app
  step "Starting with your Mac"
  if ask_yes "Open ClaudeOS when you log in to this Mac?" y; then open_at_login on; else open_at_login off; fi
  step "Done"
  say "When ClaudeOS starts, macOS asks to let it use the microphone: choose Allow."
  say "To let it click and type on websites for you, allow it in System Settings, Privacy & Security, Accessibility."
  say "To go back to macOS, say \"switch to macOS\" (say \"Claude\" to come back), or press Command-H. Command-Q quits it."
  if ask_yes "Start ClaudeOS now?" y; then open "$APP"; fi
}

sign_in_again() {
  [ -x "$CO/runtime/bin/python" ] || stop "ClaudeOS isn't installed on this Mac yet. Choose Install."
  mkdir -p "$CO/logs"
  sign_in
  say "Quit ClaudeOS (Command-Q) and open it again to use the new sign-in."
}

choose_login() {
  local now="doesn't open"
  [ -f "$AGENT" ] && now="opens"
  say "Right now ClaudeOS $now when you log in."
  ask_choice "What should happen when you log in to this Mac?" \
    "Open ClaudeOS" \
    "Just macOS (ClaudeOS stays installed; open it from Applications when you want)"
  if [ "$CHOICE" = 1 ]; then open_at_login on; good "ClaudeOS opens when you log in."
  else open_at_login off; good "The Mac starts in macOS as usual. ClaudeOS is still in Applications."; fi
}

remove() {
  step "Remove ClaudeOS from this Mac"
  if [ ! -d "$CO" ] && [ ! -d "$APP" ] && [ ! -f "$AGENT" ]; then
    say "ClaudeOS isn't installed on this Mac. Nothing was changed."
    exit 0
  fi
  say "This deletes the ClaudeOS app, its Python and voice, its sign-in and its website sign-ins and cookies."
  say "macOS, your apps and your files stay as they are."
  printf 'Type REMOVE to remove ClaudeOS (anything else keeps it): '
  local a
  read -r a || a=""
  if [ "$a" != "REMOVE" ] && [ "$a" != "remove" ]; then
    log "Q: remove -> (something else)"
    say "Kept ClaudeOS. Nothing was changed."
    exit 0
  fi
  log "Q: remove -> REMOVE"
  local keep=0
  if ask_yes "Keep what ClaudeOS remembers about you, its notes and your conversations (in $CO)?" n; then keep=1; fi
  stop_running
  # Sign ClaudeOS's own copy of the Claude engine out, which takes its sign-in out of the Keychain too.
  local cli signed_out=1
  if [ -d "$CO/.engine" ]; then
    signed_out=0
    cli="$(engine)"
    if [ -n "$cli" ] && [ -x "$cli" ] && env -u CLAUDE_CODE_OAUTH_TOKEN -u ANTHROPIC_API_KEY \
        CLAUDE_CONFIG_DIR="$CO/.engine" "$cli" auth logout >/dev/null 2>&1; then
      signed_out=1
    fi
  fi
  open_at_login off
  rm -rf "$APP"
  for d in "Library/WebKit/com.claudeos.app" "Library/Caches/com.claudeos.app" "Library/HTTPStorages/com.claudeos.app" \
           "Library/Saved Application State/com.claudeos.app.savedState"; do
    rm -rf "${ROOT:?}/$d"
  done
  if [ $keep = 1 ]; then
    for d in app runtime python models secrets .engine .setup web index undo logs; do rm -rf "${CO:?}/$d"; done
    rm -f "$CO/settings.json" "$CO/state.json" "$CO/launch.conf"
    good "ClaudeOS is removed. What it remembered, its notes and your conversations are still in $CO."
  else
    rm -rf "${CO:?}"
    good "ClaudeOS is removed, with everything it kept."
  fi
  if [ $signed_out = 0 ]; then
    warn "ClaudeOS's sign-in for typing from your phone may still be saved in this Mac's Keychain."
  fi
}

main() {
  printf '\n\033[1mClaudeOS Setup\033[0m\n'
  say "ClaudeOS fills this Mac's screen and you talk to it, or type to it from the Claude app on your phone."
  say "macOS, your apps and your files stay as they are."
  local first="Install ClaudeOS on this Mac"
  [ -x "$CO/runtime/bin/python" ] && first="Update ClaudeOS"
  ask_choice "What would you like to do?" "$first" "Sign ClaudeOS in to Claude again" \
    "Choose whether ClaudeOS opens when you log in" "Remove ClaudeOS from this Mac"
  case "$CHOICE" in
    1) install ;;
    2) sign_in_again ;;
    3) choose_login ;;
    4) remove ;;
  esac
}

main "$@"
