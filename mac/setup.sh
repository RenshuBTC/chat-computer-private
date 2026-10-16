#!/bin/bash
# ClaudeOS Setup for Mac. Run it in Terminal:
#     bash ~/Downloads/ClaudeOS/mac/setup.sh
# (or double-click "ClaudeOS Setup.command" next to this folder).
#
# ClaudeOS on a Mac is its own system, started instead of macOS (like on a PC, where it is started instead of
# Windows). macOS stays, with your apps and files, one restart away. It works on Macs with Apple silicon M1,
# M2 and M3 (not M3 Ultra): ClaudeOS is built on Fedora Asahi Remix, and Asahi Linux doesn't support newer
# chips yet.
#
# Installing it:
#   1. checks this Mac, signs in to Claude (the official Claude program from Anthropic's Agent SDK), notes
#      the Wi-Fi, and asks which folders to bring over: ClaudeOS can't open macOS's own files (Apple encrypts
#      them for macOS only), so setup copies them now, onto a drive called ClaudeOS that both systems open;
#   2. downloads the Asahi installer and Fedora Asahi Remix's Minimal image (mac/manifest.sh: pinned, checked
#      against SHA-256 fingerprints) and runs the installer, answering its questions (mac/drive-installer.exp):
#      it makes room next to macOS and installs Fedora there; you type your Mac's password once;
#   3. makes the ClaudeOS drive in the space left over and copies your folders onto it;
#   4. leaves ClaudeOS's first start on ClaudeOS's EFI partition (mac/esp): your account, Wi-Fi and sign-in;
#   5. shows the one step Apple requires in the Mac's startup options, and shuts the Mac down.
# On its first start ClaudeOS sets itself up from there and installs the rest with the internet.
#
# It can also send more folders to ClaudeOS, sign ClaudeOS in again, choose what starts when the Mac turns on,
# and remove ClaudeOS completely. Written for the bash that comes with macOS (3.2).
# Messages and logs go to the person's own log on purpose, also from commands run with sudo; a failed check
# stops setup (stop exits), so "A && B || stop" can't fall through.
# shellcheck disable=SC2015,SC2024,SC1091
set -u

ROOT="${CLAUDEOS_MAC_ROOT:-$HOME}"             # tests point these at a pretend Mac
WORK="${CLAUDEOS_SETUP_WORK:-$ROOT/.claudeos-setup}"
PLAN_ONLY="${CLAUDEOS_PLAN_ONLY:-0}"           # 1: check and plan, change nothing
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SRC="$(cd "$HERE/.." && pwd)"                  # the folder you downloaded (it has claudeos/, linux/ and mac/)
LOG="$WORK/setup.log"
# shellcheck source=manifest.sh
. "${CLAUDEOS_MANIFEST:-$HERE/manifest.sh}"

GB=1000000000
MIN_MACOS_FREE=$((38 * GB))      # the Asahi installer always leaves macOS this much free space, for its updates
CLAUDEOS_MIN=$((30 * GB))        # ClaudeOS's own space: Fedora, ClaudeOS, its voice, and room to work
CLAUDEOS_WANT=$((60 * GB))
DRIVE_ROOM=$((10 * GB))          # room on the ClaudeOS drive beyond what's copied there now
OS_NAME="ClaudeOS"               # the name in the Mac's startup options (asahi-bless in ClaudeOS uses it too)
DRIVE_NAME="ClaudeOS"            # the drive both systems open
FOLDERS="Desktop Documents Downloads Pictures Movies Music"

# ------------------------------------------------------------------ talking to the person
# Anything shaped like a Claude sign-in is blanked out before it is written to the log.
log() {
  if [ -d "$WORK" ]; then
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
    # Only real answers are logged: anything else could be something secret typed in the wrong place.
    log "Q: $q -> (not y or n)"
    printf 'Please type y for yes or n for no.\n'
  done
}

ask_choice() {   # ask_choice QUESTION OPTION... ; sets CHOICE to the number picked (1-based)
  local q="$1" i=1 a o
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

ask_word() {   # ask_word PROMPT WORD: true if the person typed WORD
  local a
  printf '%s ' "$1"
  read -r a || a=""
  if [ "$a" = "$2" ]; then log "Q: $1 -> $2"; return 0; fi
  log "Q: $1 -> (something else)"
  return 1
}

read_secret() {   # read_secret PROMPT: a hidden answer into SECRET (never logged)
  local saved=""
  printf '%s ' "$1"
  saved="$(stty -g 2>/dev/null)" && stty -echo 2>/dev/null
  read -r SECRET || SECRET=""
  [ -n "$saved" ] && stty "$saved" 2>/dev/null
  printf '\n'
}

# Reads what's pasted, hidden, however many lines it has, into PASTED, up to the Enter that follows it.
# Claude's sign-in code is longer than Terminal's usual width and Claude Code wraps it with real line breaks,
# so a copy of it can hold two or more lines. Terminal marks where a paste starts and ends when asked to
# (bracketed paste), so line breaks inside the paste don't end it, and the Enter pressed after it does.
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
    [ -z "$CHUNK" ] && break
    all="$all$CHUNK"
    if [ "${all#*"$start"}" != "$all" ]; then
      case "${all##*"$start"}" in *"$end"*) ;; *) continue ;; esac
    fi
    rest="${all##*"$end"}"
    case "$rest" in *$'\n'*|*$'\r'*) ;; *) continue ;; esac
    stty min 0 time 5
    read_chunk
    stty min 1 time 0
    [ -z "$CHUNK" ] && break
    all="$all$CHUNK"
  done
  restore_terminal
  PASTED="$all"
}

# ------------------------------------------------------------------ small helpers
plist() {   # plist FILE KEY.PATH: one value from a property list (diskutil -plist output)
  plutil -extract "$2" raw -o - "$1" 2>/dev/null
}

size_text() {   # size_text BYTES: like "123.4 GB"
  awk -v b="$1" 'BEGIN { if (b >= 1e12) printf "%.1f TB", b / 1e12; else if (b >= 1e9) printf "%.1f GB", b / 1e9;
                         else printf "%d MB", b / 1e6 }'
}

b64() { printf '%s' "$1" | base64 | tr -d '\n'; }

fetch() {   # fetch URL FILE SHA256 WHAT: download (resuming) and check the fingerprint
  local got
  if [ -f "$2" ]; then
    got="$(shasum -a 256 "$2" | awk '{print $1}')"
    [ "$got" = "$3" ] && return 0
  fi
  say "Downloading $4..."
  curl -fL --retry 3 -C - --progress-bar -o "$2" "$1" \
    || curl -fL --retry 3 --progress-bar -o "$2" "$1" \
    || stop "The download of $4 failed. Check the internet connection and run setup again."
  got="$(shasum -a 256 "$2" | awk '{print $1}')"
  if [ "$got" != "$3" ]; then
    rm -f "$2"
    stop "The download of $4 didn't match its known fingerprint (got $got). Nothing was changed."
  fi
}

# ------------------------------------------------------------------ this Mac
system_disk() {   # the disk macOS is on, like disk0
  local store
  diskutil info -plist / > "$WORK/root.plist" 2>/dev/null || return 1
  store="$(plist "$WORK/root.plist" APFSPhysicalStores.0.APFSPhysicalStore)"
  printf '%s' "${store%s*}"
}

check_mac() {
  step "Checking this Mac"
  [ "$(uname -s)" = Darwin ] || stop "Run this on the Mac, in macOS."
  # A Terminal running under Rosetta says x86_64 on Apple silicon too; ask the hardware.
  if [ "$(sysctl -n hw.optional.arm64 2>/dev/null)" != 1 ]; then
    stop "ClaudeOS starts on its own only on Macs with Apple silicon (M1, M2 or M3). This Mac has an Intel processor."
  fi
  CHIP="$(sysctl -n machdep.cpu.brand_string 2>/dev/null)"
  case "$CHIP" in
    "Apple M1"*|"Apple M2"*) ;;
    "Apple M3 Ultra"*)
      stop "This Mac has an $CHIP. ClaudeOS works on M1, M2 and M3 Macs, but not yet on the Mac Studio with M3 Ultra: Asahi Linux, which it's built on, doesn't support it yet." ;;
    "Apple M3"*)
      say "This Mac has an $CHIP. On M3 Macs, Asahi Linux doesn't yet have fast graphics, sleep or HDMI;"
      say "ClaudeOS works, but the Mac stays awake when it's on, and screens connect over USB-C only." ;;
    *)
      [ -n "$CHIP" ] || CHIP="Apple chip setup doesn't know"
      stop "This Mac has an $CHIP. ClaudeOS can start on its own only on Macs with M1, M2 or M3: Asahi Linux, which it's built on, doesn't support newer chips yet." ;;
  esac
  if [ "$(sysctl -n kern.hv_vmm_present 2>/dev/null)" = 1 ] && [ "$PLAN_ONLY" != 1 ]; then
    stop "This is a virtual Mac. ClaudeOS installs on a real Mac's own disk."
  fi
  MACOS="$(sw_vers -productVersion 2>/dev/null)"
  local major minor
  major="${MACOS%%.*}"
  minor="${MACOS#*.}"; minor="${minor%%.*}"
  case "$major$minor" in ''|*[!0-9]*) stop "This doesn't look like a Mac (no macOS version found)." ;; esac
  if [ "$major" -lt 13 ] || { [ "$major" -eq 13 ] && [ "$minor" -lt 5 ]; }; then
    stop "ClaudeOS needs macOS 13.5 or newer to install; this Mac has macOS $MACOS. Update it in System Settings, then run setup again."
  fi
  if ! id -Gn 2>/dev/null | tr ' ' '\n' | grep -qx admin; then
    stop "Your account on this Mac isn't an administrator. Log in as one, then run setup again."
  fi
  say "macOS $MACOS on an $CHIP."
}

# ClaudeOS's pieces on this Mac, from diskutil: its start-up volume ("ClaudeOS"), its EFI partition
# ("EFI - CLAUD", the Asahi installer's name for it), the two Linux partitions after it, and the ClaudeOS drive.
find_install() {
  FOUND_STUB="" FOUND_ESP="" FOUND_BOOT="" FOUND_LINUX="" FOUND_DRIVE=""
  local d="$1" i=0 id content name after=""
  diskutil list -plist "$d" > "$WORK/parts.plist" 2>/dev/null || return 1
  while id="$(plist "$WORK/parts.plist" "AllDisksAndPartitions.0.Partitions.$i.DeviceIdentifier")" && [ -n "$id" ]; do
    content="$(plist "$WORK/parts.plist" "AllDisksAndPartitions.0.Partitions.$i.Content")"
    name="$(plist "$WORK/parts.plist" "AllDisksAndPartitions.0.Partitions.$i.VolumeName")"
    case "$content" in
      Apple_APFS)
        if diskutil apfs list -plist "$id" 2>/dev/null | grep -q "<string>$OS_NAME</string>"; then FOUND_STUB="$id"; fi
        after="" ;;
      EFI)
        case "$(printf '%s' "$name" | tr '[:lower:]' '[:upper:]')" in "EFI - CLAUD"*) FOUND_ESP="$id"; after=esp ;; *) after="" ;; esac ;;
      "Linux Filesystem"|"0FC63DAF-8483-4772-8E79-3D69D8477DE4")
        if [ "$after" = esp ]; then FOUND_BOOT="$id"; after=boot
        elif [ "$after" = boot ]; then FOUND_LINUX="$id"; after=linux
        else after=""; fi ;;
      *)
        if [ "$name" = "$DRIVE_NAME" ] && [ "$after" = linux ]; then FOUND_DRIVE="$id"; fi
        after="" ;;
    esac
    i=$((i + 1))
  done
  [ -n "$FOUND_STUB$FOUND_ESP$FOUND_LINUX$FOUND_DRIVE" ]
}

read_disk() {
  diskutil info -plist / > "$WORK/root.plist" 2>/dev/null || stop "diskutil couldn't describe macOS's disk."
  CONTAINER="$(plist "$WORK/root.plist" APFSContainerReference)"
  STORE="$(plist "$WORK/root.plist" APFSPhysicalStores.0.APFSPhysicalStore)"
  DISK="${STORE%s*}"
  case "$CONTAINER:$STORE:$DISK" in
    disk[0-9]*:disk[0-9]*s[0-9]*:disk[0-9]*) ;;
    *) stop "Setup couldn't tell which disk macOS is on ($CONTAINER, $STORE)." ;;
  esac
  diskutil apfs list -plist "$CONTAINER" > "$WORK/container.plist" 2>/dev/null || stop "diskutil couldn't describe macOS's container."
  TOTAL="$(plist "$WORK/container.plist" Containers.0.CapacityCeiling)"
  FREE="$(plist "$WORK/container.plist" Containers.0.CapacityFree)"
  diskutil apfs resizeContainer "$STORE" limits -plist > "$WORK/limits.plist" 2>/dev/null \
    || stop "diskutil couldn't say how far macOS's partition can shrink."
  local pref
  pref="$(plist "$WORK/limits.plist" MinimumSizePreferred)"
  case "$TOTAL$FREE$pref" in ''|*[!0-9]*) stop "diskutil gave sizes setup can't read." ;; esac
  # As the Asahi installer works it out: macOS keeps what it uses, plus 38 GB free, and never less than what
  # diskutil prefers (Time Machine's snapshots count there), and a little more to be safe.
  MACOS_MIN=$((TOTAL - FREE + MIN_MACOS_FREE))
  [ "$pref" -gt "$MACOS_MIN" ] && MACOS_MIN="$pref"
  MACOS_MIN=$((MACOS_MIN + GB))
  AVAIL=$((TOTAL - MACOS_MIN))
  [ "$AVAIL" -lt 0 ] && AVAIL=0
  log "disk: container=$CONTAINER store=$STORE total=$TOTAL free=$FREE preferred-min=$pref available=$AVAIL"
}

# ------------------------------------------------------------------ what comes over from macOS
folder_bytes() {   # the size of a folder, as macOS reports file sizes (files kept only in iCloud count too)
  du -A -sk "$1" 2>/dev/null | awk '{ printf "%d", $1 * 1024 }'
}

choose_folders() {
  step "Your files"
  say "ClaudeOS can't open macOS's files itself: Apple encrypts them so only macOS can. So setup copies the folders"
  say "you choose onto a drive called ClaudeOS that both systems open; macOS keeps its own copies. Anything you save"
  say "on that drive in ClaudeOS is in macOS too, and you can send more later (run setup again)."
  say "macOS may ask whether Terminal can open your Desktop, Documents and Downloads folders: choose OK."
  say ""
  local f b names="" sizes="" total=0
  for f in $FOLDERS; do
    [ -d "$ROOT/$f" ] || continue
    b="$(folder_bytes "$ROOT/$f")"
    [ -n "$b" ] || b=0
    names="$names $f"
    sizes="$sizes $b"
    total=$((total + b))
    printf '  %-10s %s\n' "$f" "$(size_text "$b")"
  done
  CHOSEN=""
  CHOSEN_BYTES=0
  if ask_yes "Bring all of these to ClaudeOS ($(size_text "$total"))?" y; then
    CHOSEN="$names"
    CHOSEN_BYTES="$total"
  else
    # shellcheck disable=SC2086
    set -- $sizes
    for f in $names; do
      if ask_yes "  Bring $f ($(size_text "$1"))?" y; then
        CHOSEN="$CHOSEN $f"
        CHOSEN_BYTES=$((CHOSEN_BYTES + $1))
      fi
      shift
    done
  fi
  log "folders:$CHOSEN ($CHOSEN_BYTES bytes)"
}

# ------------------------------------------------------------------ signing in to Claude
claude_program() {   # the official Claude program, from Anthropic's Agent SDK
  if [ -n "${CLAUDEOS_ENGINE_CLI:-}" ]; then CLAUDE="$CLAUDEOS_ENGINE_CLI"; return 0; fi
  CLAUDE="$WORK/claude/claude_agent_sdk/_bundled/claude"
  [ -x "$CLAUDE" ] && return 0
  mkdir -p "$WORK/claude"
  fetch "$CLAUDE_WHEEL_URL" "$WORK/claude/agent-sdk.whl" "$CLAUDE_WHEEL_SHA256" "the Claude program (about 100 MB)"
  (cd "$WORK/claude" && unzip -o -q agent-sdk.whl 'claude_agent_sdk/_bundled/claude') || stop "Couldn't unpack the Claude program."
  chmod 755 "$CLAUDE"
}

sign_in() {   # sets TOKEN, and LOGIN (the full sign-in for the phone) or ""
  step "Sign in to Claude"
  local tmp raw try=1
  TOKEN="" LOGIN=""
  claude_program
  say "Your browser opens to sign in to Claude. Use the account with your Claude subscription."
  say "When it finishes, this window shows a long code starting with sk-ant-. It may run over two lines:"
  say "copy all of it."
  tmp="$(mktemp -d)"
  CLAUDE_CONFIG_DIR="$tmp" "$CLAUDE" setup-token
  rm -rf "$tmp"
  while [ $try -le 3 ]; do
    printf 'Paste the code here (it stays hidden), then press Enter: '
    read_paste
    raw="$PASTED"
    printf '\n'
    TOKEN="$(printf '%s' "$raw" | tr -d ' \t\r\n"'"'" | grep -oE 'sk-ant-(oat01|api03)-[A-Za-z0-9_-]{20,}' | head -n 1)"
    [ -n "$TOKEN" ] && break
    warn "That doesn't look like the code. It starts with sk-ant- and is about 100 characters long."
    try=$((try + 1))
  done
  [ -n "$TOKEN" ] || stop "No valid Claude sign-in code was entered."
  good "Got it."
  say ""
  say "Besides talking, you can type to ClaudeOS in the Claude app on your phone: the phone is its keyboard."
  if ask_yes "Set up typing from your phone? (your browser opens once more)" y; then
    tmp="$(mktemp -d)"
    CLAUDE_CONFIG_DIR="$tmp" "$CLAUDE" auth login --claudeai
    if [ -f "$tmp/.credentials.json" ]; then
      LOGIN="$(cat "$tmp/.credentials.json")"
      good "Done. In the Claude app on your phone, tap Code, then ClaudeOS."
    else
      warn "That sign-in didn't finish. ClaudeOS works without it; run setup again and choose to sign in again to add it."
    fi
    rm -rf "$tmp"
  fi
}

# ------------------------------------------------------------------ Wi-Fi for ClaudeOS's first start
wifi_device() {
  networksetup -listallhardwareports 2>/dev/null | awk '/^Hardware Port: (Wi-Fi|AirPort)/ { getline; print $2; exit }'
}

current_wifi() {
  local dev ssid
  dev="$(wifi_device)"
  [ -n "$dev" ] || return 0
  ssid="$(networksetup -getairportnetwork "$dev" 2>/dev/null | sed -n 's/^Current Wi-Fi Network: //p')"
  if [ -z "$ssid" ]; then
    ssid="$(ipconfig getsummary "$dev" 2>/dev/null | sed -n 's/^ *SSID : //p' | head -n 1)"
  fi
  case "$ssid" in *redacted*) ssid="" ;; esac
  printf '%s' "$ssid"
}

ask_wifi() {   # sets WIFI_SSID and WIFI_PSK
  step "Wi-Fi for ClaudeOS"
  say "ClaudeOS finishes installing itself the first time it starts, from the internet, so it needs your Wi-Fi."
  local seen a
  seen="$(current_wifi)"
  WIFI_SSID=""
  if [ -n "$seen" ]; then
    printf 'Wi-Fi name (Enter = %s): ' "$seen"
  else
    printf 'Wi-Fi name: '
  fi
  read -r a || a=""
  [ -z "$a" ] && a="$seen"
  WIFI_SSID="$a"
  [ -n "$WIFI_SSID" ] || stop "ClaudeOS needs a Wi-Fi network for its first start."
  log "Wi-Fi: $WIFI_SSID"
  read_secret "Password for \"$WIFI_SSID\" (it stays hidden; Enter = look it up in this Mac's keychain, which may ask for your Mac's password):"
  WIFI_PSK="$SECRET"
  SECRET=""
  if [ -z "$WIFI_PSK" ]; then
    WIFI_PSK="$(security find-generic-password -D 'AirPort network password' -a "$WIFI_SSID" -w 2>/dev/null)"
    if [ -n "$WIFI_PSK" ]; then
      good "Found it in the keychain."
    elif ! ask_yes "No password found. Is \"$WIFI_SSID\" an open network (no password)?" n; then
      stop "ClaudeOS needs the Wi-Fi password for its first start. Run setup again and type it."
    fi
  fi
}

# ------------------------------------------------------------------ the space
plan_space() {
  step "Where ClaudeOS goes"
  read_disk
  DRIVE_BYTES=$((CHOSEN_BYTES + CHOSEN_BYTES / 10 + DRIVE_ROOM))
  CLAUDEOS_BYTES="$CLAUDEOS_WANT"
  if [ $((CLAUDEOS_BYTES + DRIVE_BYTES)) -gt "$AVAIL" ]; then
    CLAUDEOS_BYTES=$((AVAIL - DRIVE_BYTES))
  fi
  if [ "$CLAUDEOS_BYTES" -lt "$CLAUDEOS_MIN" ]; then
    say "macOS can give ClaudeOS $(size_text "$AVAIL") and still keep 38 GB free for its updates."
    if [ "$AVAIL" -ge $((CLAUDEOS_MIN + DRIVE_ROOM)) ] && [ "$CHOSEN_BYTES" -gt 0 ]; then
      stop "That's not enough for ClaudeOS ($(size_text "$CLAUDEOS_MIN")) and the $(size_text "$CHOSEN_BYTES") of folders. Run setup again and bring fewer folders, or free up space in macOS."
    fi
    stop "ClaudeOS needs $(size_text $((CLAUDEOS_MIN + DRIVE_ROOM))). Free up space in macOS, then run setup again."
  fi
  NEED=$((CLAUDEOS_BYTES + DRIVE_BYTES))
  MACOS_NEW=$((TOTAL - NEED))
  say "macOS's partition is $(size_text "$TOTAL") with $(size_text "$FREE") free."
  say "ClaudeOS gets $(size_text "$CLAUDEOS_BYTES"), and the ClaudeOS drive $(size_text "$DRIVE_BYTES")."
}

# ------------------------------------------------------------------ the installer
prepare_installer() {
  step "Downloading"
  mkdir -p "$WORK/os"
  fetch "$ASAHI_INSTALLER_URL" "$WORK/asahi-installer.tar.gz" "$ASAHI_INSTALLER_SHA256" "the Asahi installer"
  INST="$WORK/asahi-install"
  rm -rf "$INST"
  mkdir -p "$INST"
  tar -xzf "$WORK/asahi-installer.tar.gz" -C "$INST" || stop "Couldn't unpack the Asahi installer."
  cp "$HERE/installer_data.json" "$INST/installer_data.json"
  # Fedora's image. ClaudeOS's icon for the Mac's startup options goes into its package (the installer takes it
  # from there) once the package is checked, with the Python that comes with the installer. That changes the
  # package's fingerprint, so the new one is noted next to it: running setup again after a stop doesn't download
  # it again.
  local pkg="$WORK/os/$FEDORA_PACKAGE" noted="" py="$INST/Frameworks/Python.framework/Versions/3.13/bin/python3.13"
  [ -f "$pkg.with-icon" ] && noted="$(cat "$pkg.with-icon")"
  if [ -n "$noted" ] && [ -f "$pkg" ] && [ "$(shasum -a 256 "$pkg" | awk '{print $1}')" = "$noted" ]; then
    say "Fedora Asahi Remix is downloaded already."
    return 0
  fi
  rm -f "$pkg.with-icon"
  fetch "$FEDORA_PACKAGE_URL" "$pkg" "$FEDORA_PACKAGE_SHA256" "Fedora Asahi Remix (about 1.1 GB)"
  if DYLD_LIBRARY_PATH="$INST/Frameworks/Python.framework/Versions/Current/lib" DYLD_FRAMEWORK_PATH="$INST/Frameworks" \
       "$py" - "$pkg" "$HERE/claudeos.icns" <<'EOF'
import sys, zipfile
with zipfile.ZipFile(sys.argv[1], "a") as z:
    if "claudeos.icns" not in z.namelist():
        z.write(sys.argv[2], "claudeos.icns")
EOF
  then
    shasum -a 256 "$pkg" | awk '{print $1}' > "$pkg.with-icon"
  else
    # Without it in the package, the installer must not look for it: it shows its own icon instead.
    if [ "$(shasum -a 256 "$pkg" | awk '{print $1}')" != "$FEDORA_PACKAGE_SHA256" ]; then
      rm -f "$pkg"
      stop "Fedora's package was damaged while ClaudeOS's icon was added to it. Run setup again (it downloads it again)."
    fi
    warn "ClaudeOS's icon couldn't be added; the startup options show the installer's own."
    sed '/"icon":/d' "$HERE/installer_data.json" > "$INST/installer_data.json"
  fi
}

run_installer() {
  step "Installing ClaudeOS next to macOS"
  say "The Asahi installer makes room next to macOS and installs ClaudeOS's base (Fedora Linux) there. Setup"
  say "answers its questions. When it asks for your Mac's password, type it (it stays hidden) and press Enter."
  say "It takes about 10 to 20 minutes; your Mac may seem to freeze while macOS's partition shrinks. That's normal."
  say "First, your Mac's password for setup itself:"
  sudo -v || stop "Setup needs your Mac's password to install ClaudeOS."
  local code=0
  (
    cd "$INST" || exit 6
    export REPO_BASE="$WORK" DISTRO="ClaudeOS" DISTRO_DOCS="https://docs.fedoraproject.org/en-US/fedora-asahi-remix/"
    unset REPORT REPORT_TAG EXPERT
    # shellcheck disable=SC2086
    CLAUDEOS_MACOS_SIZE="${MACOS_NEW}B" CLAUDEOS_OS_SIZE="${CLAUDEOS_BYTES}B" \
      ${CLAUDEOS_EXPECT:-expect} -f "$HERE/drive-installer.exp" -- ${CLAUDEOS_INSTALLER_CMD:-caffeinate -dis sudo -E ./install.sh}
  ) || code=$?
  [ -f "$INST/installer.log" ] && cp "$INST/installer.log" "$WORK/asahi-installer.log"
  case "$code" in
    0) ;;
    2) stop "macOS's partition couldn't be made smaller. If Time Machine keeps local snapshots, see https://alx.sh/tmcleanup; then run setup again." ;;
    3) stop "There wasn't enough free space after all. Free up space in macOS, then run setup again." ;;
    5) stop "macOS needs updating first. Update it in System Settings, then run setup again." ;;
    *) stop "The Asahi installer didn't finish (see above; its log is $WORK/asahi-installer.log). If it made room or partitions, run setup again and choose \"Remove ClaudeOS from this Mac\" to clean up." ;;
  esac
  # Which partitions it made, from its log: "Installing partition {'name': 'EFI', ...} -> disk0s4".
  EFI_DEV="$(sed -n "s/.*Installing partition {'name': 'EFI'.* -> \(disk[0-9]*s[0-9]*\)\$/\1/p" "$WORK/asahi-installer.log" | tail -n 1)"
  ROOT_DEV="$(sed -n "s/.*Installing partition {'name': 'Root'.* -> \(disk[0-9]*s[0-9]*\)\$/\1/p" "$WORK/asahi-installer.log" | tail -n 1)"
  [ -n "$EFI_DEV" ] && [ -n "$ROOT_DEV" ] || stop "Setup couldn't find the partitions the installer made (its log: $WORK/asahi-installer.log)."
  good "ClaudeOS's base is installed."
}

# ------------------------------------------------------------------ the ClaudeOS drive
make_drive() {
  step "The ClaudeOS drive"
  sudo diskutil addPartition "$ROOT_DEV" ExFAT "$DRIVE_NAME" 0 >> "$LOG" 2>&1 \
    || stop "Couldn't make the ClaudeOS drive in the space left over (details: $LOG)."
  find_install "$DISK" || true
  [ -n "$FOUND_DRIVE" ] || stop "Setup made the ClaudeOS drive but can't find it now."
  diskutil info -plist "$FOUND_DRIVE" > "$WORK/drive.plist" 2>/dev/null
  DRIVE_PARTUUID="$(plist "$WORK/drive.plist" DiskUUID)"
  DRIVE_MOUNT="$(plist "$WORK/drive.plist" MountPoint)"
  if [ -z "$DRIVE_MOUNT" ]; then
    diskutil mount "$FOUND_DRIVE" >> "$LOG" 2>&1
    diskutil info -plist "$FOUND_DRIVE" > "$WORK/drive.plist" 2>/dev/null
    DRIVE_MOUNT="$(plist "$WORK/drive.plist" MountPoint)"
  fi
  [ -n "$DRIVE_MOUNT" ] && [ -d "$DRIVE_MOUNT" ] || stop "Couldn't open the ClaudeOS drive."
  # Spotlight and Finder's own files would only clutter it.
  sudo mdutil -i off "$DRIVE_MOUNT" >> "$LOG" 2>&1 || true
  touch "$DRIVE_MOUNT/.metadata_never_index" 2>/dev/null
  mkdir -p "$DRIVE_MOUNT/.claudeos"
  good "The ClaudeOS drive is ready ($(size_text "$DRIVE_BYTES"))."
}

copy_folders() {   # copy_folders DEST FOLDER...: copies, without macOS's extra file data (no ._ files)
  local dest="$1" f code
  shift
  for f in "$@"; do
    say "Copying $f..."
    code=0
    COPYFILE_DISABLE=1 rsync -rt --exclude '.DS_Store' --exclude '*.photoslibrary' --exclude '.Trash*' \
      "$ROOT/$f/" "$dest/$f/" >> "$LOG" 2>&1 || code=$?
    case "$code" in
      0) ;;
      23|24) warn "Some files in $f couldn't be copied (details: $LOG)." ;;
      *) warn "Copying $f stopped (details: $LOG). It may not all be there." ;;
    esac
  done
}

# ------------------------------------------------------------------ ClaudeOS's first start
write_first_start() {
  step "Preparing ClaudeOS's first start"
  local esp_mount p
  diskutil mount "$EFI_DEV" >> "$LOG" 2>&1
  diskutil info -plist "$EFI_DEV" > "$WORK/esp.plist" 2>/dev/null
  esp_mount="$(plist "$WORK/esp.plist" MountPoint)"
  [ -n "$esp_mount" ] && [ -f "$esp_mount/EFI/fedora/grub.cfg" ] || stop "Couldn't open ClaudeOS's EFI partition ($EFI_DEV)."
  p="$WORK/payload/claudeos"
  rm -rf "$WORK/payload"
  mkdir -p "$p/secrets"
  chmod 700 "$WORK/payload" "$p/secrets"
  # ClaudeOS itself: what runs on Linux (no setup tools for Windows or macOS).
  # (No extended attributes: macOS adds its own to files, which Fedora's tar would only warn about.)
  COPYFILE_DISABLE=1 tar --no-xattrs --exclude '__pycache__' --exclude '.DS_Store' -czf "$p/app.tar.gz" -C "$SRC" \
    claudeos linux requirements-linux.txt pyproject.toml README.md ARCHITECTURE.md 2>> "$LOG" \
    || stop "Couldn't pack ClaudeOS."
  # Written afresh, not copied: a copy would carry the download's quarantine mark, which FAT keeps as a ._ file.
  cat "$HERE/esp/firstboot.sh" > "$p/firstboot.sh"
  ( umask 077
    {
      printf 'USER_NAME=%s\n' "$(b64 "$ACCOUNT")"
      printf 'REAL_NAME=%s\n' "$(b64 "$REAL_NAME")"
      printf 'RECOVERY_PASSWORD=%s\n' "$(b64 "$RECOVERY")"
      printf 'TIMEZONE=%s\n' "$(b64 "$TZONE")"
      printf 'INSTALL_ID=%s\n' "$(b64 "$INSTALL_ID")"
      printf 'MAC_FILES_PARTUUID=%s\n' "$(b64 "$DRIVE_PARTUUID")"
      printf 'WIFI_SSID=%s\n' "$(b64 "$WIFI_SSID")"
      printf 'WIFI_PSK=%s\n' "$(b64 "$WIFI_PSK")"
    } > "$p/setup.env"
    printf '%s\n' "$TOKEN" > "$p/secrets/claude_token"
    [ -n "$LOGIN" ] && printf '%s\n' "$LOGIN" > "$p/secrets/claude_login.json"
  )
  printf '{\n  "user_name": "%s",\n  "timezone": "%s",\n  "locale": "en_US.UTF-8"\n}\n' \
    "$(printf '%s' "$REAL_NAME" | sed 's/\\/\\\\/g; s/"/\\"/g')" "$TZONE" > "$p/settings.json"
  sudo mkdir -p "$esp_mount/claudeos" || stop "Couldn't write to ClaudeOS's EFI partition."
  sudo cp "$esp_mount/EFI/fedora/grub.cfg" "$esp_mount/claudeos/fedora-grub.cfg"
  sudo cp -R "$p/." "$esp_mount/claudeos/" || stop "Couldn't write to ClaudeOS's EFI partition."
  sed -e "s/@KERNEL@/$FEDORA_KERNEL/g" -e "s/@BOOT_UUID@/$FEDORA_BOOT_UUID/g" -e "s/@ROOT_UUID@/$FEDORA_ROOT_UUID/g" \
    "$HERE/esp/grub-firststart.cfg" > "$WORK/grub-firststart.cfg"
  sudo cp "$WORK/grub-firststart.cfg" "$esp_mount/EFI/fedora/grub.cfg" || stop "Couldn't write to ClaudeOS's EFI partition."
  sync
  rm -rf "$WORK/payload"
  diskutil unmount "$EFI_DEV" >> "$LOG" 2>&1
  good "ClaudeOS will set itself up when it first starts."
}

# ------------------------------------------------------------------ the four things setup does
install() {
  check_mac
  if find_install "$(system_disk)"; then
    stop "ClaudeOS is already on this Mac. To install it again, run setup and choose \"Remove ClaudeOS from this Mac\" first."
  fi
  if ! curl -sSfI --max-time 20 "https://fedora-asahi-remix.org/" -o /dev/null 2>/dev/null; then
    stop "This Mac can't reach the internet (fedora-asahi-remix.org). Connect to Wi-Fi, then run setup again."
  fi
  choose_folders
  plan_space
  REAL_NAME="$(id -F 2>/dev/null)"
  [ -n "$REAL_NAME" ] || REAL_NAME="$(id -un)"
  ACCOUNT="$(id -un | tr '[:upper:]' '[:lower:]' | tr -c 'a-z0-9_-' '-' | sed 's/^[^a-z_]*//; s/-*$//' | cut -c1-30)"
  [ -n "$ACCOUNT" ] || ACCOUNT="claudeos"
  TZONE="$(readlink /etc/localtime 2>/dev/null | sed -n 's|.*zoneinfo/||p')"
  [ -n "$TZONE" ] || TZONE="UTC"
  say "Name: $REAL_NAME   Account: $ACCOUNT   Time zone: $TZONE"
  if [ "$PLAN_ONLY" = 1 ]; then
    step "Plan only"
    say "Nothing was changed. setup would shrink macOS's partition ($STORE) to $(size_text "$MACOS_NEW"),"
    say "give ClaudeOS $(size_text "$CLAUDEOS_BYTES") and the ClaudeOS drive $(size_text "$DRIVE_BYTES"), and copy:${CHOSEN:- nothing}."
    printf 'PLAN store=%s macos_new=%s claudeos=%s drive=%s folders=%s\n' "$STORE" "$MACOS_NEW" "$CLAUDEOS_BYTES" "$DRIVE_BYTES" "$CHOSEN" >> "$LOG"
    return 0
  fi
  sign_in
  ask_wifi
  RECOVERY="$(LC_ALL=C tr -dc 'a-km-z2-9' < /dev/urandom | head -c 16 | sed 's/\(....\)/\1-/g; s/-$//')"
  INSTALL_ID="$(od -An -N12 -tx1 /dev/urandom | tr -d ' \n')"

  step "Ready"
  say "This is what happens now:"
  say "  1. macOS's partition shrinks to $(size_text "$MACOS_NEW"). Your apps and files stay."
  say "  2. ClaudeOS gets $(size_text "$CLAUDEOS_BYTES") next to macOS (its base is Fedora Linux, downloaded now: 1.2 GB)."
  say "  3. The ClaudeOS drive gets $(size_text "$DRIVE_BYTES"), with copies of:${CHOSEN:- (nothing yet)}."
  say "  4. Then the Mac shuts down, and you do one short step in its startup options. Setup shows you how."
  say "ClaudeOS then starts instead of macOS. macOS stays one restart away: in ClaudeOS, say \"switch to macOS\"."
  ask_yes "Go ahead?" n || stop "Stopped at your request. Nothing was changed."

  prepare_installer
  run_installer
  make_drive
  # shellcheck disable=SC2086
  [ -n "$CHOSEN" ] && copy_folders "$DRIVE_MOUNT" $CHOSEN
  write_first_start
  TOKEN="" LOGIN="" WIFI_PSK=""
  local note="$ROOT/Documents/ClaudeOS recovery password.txt"
  mkdir -p "$ROOT/Documents"
  printf 'ClaudeOS recovery password\n\nAccount: %s\nPassword: %s\n\nYou normally never need this: ClaudeOS starts without a password. It is for repairs only.\n' \
    "$ACCOUNT" "$RECOVERY" > "$note"
  RECOVERY=""
  rm -rf "$WORK/os" "$WORK/asahi-install" "$WORK/claude"
  finish_instructions
}

finish_instructions() {
  step "One more step, in the Mac's startup options"
  say "Apple requires this step for any other system on a Mac. Take a photo of these instructions:"
  say ""
  say "  1. When the Mac has turned off, wait 25 seconds."
  say "  2. Press and HOLD the power button. Let go when \"Loading startup options...\" appears."
  say "  3. Click \"$OS_NAME\", then Continue."
  say "  4. If it asks you to \"Select a volume to recover\", choose your usual macOS (Macintosh HD), click Next"
  say "     and type your Mac's password."
  say "  5. A window \"ClaudeOS installer (second step)\" opens. Press Enter, then type your Mac's user name and"
  say "     password when asked (it asks twice). The note about the security level is about ClaudeOS only:"
  say "     macOS keeps its own."
  say "  6. At \"Installation complete! Press enter to reboot.\", press Enter."
  say ""
  say "ClaudeOS then sets itself up from your Wi-Fi (about 20 to 40 minutes), restarts once, and says hello."
  say "If the Mac starts macOS instead, start again from step 1."
  if ask_yes "Shut the Mac down now?" y; then
    sudo shutdown -h now
  else
    say "When you're ready, shut the Mac down from the Apple menu and follow the steps above."
  fi
}

send_files() {
  step "Send files to ClaudeOS"
  local mount
  find_install "$(system_disk)" || true
  [ -n "$FOUND_DRIVE" ] || stop "There's no ClaudeOS drive on this Mac. Install ClaudeOS first."
  diskutil mount "$FOUND_DRIVE" >> "$LOG" 2>&1
  diskutil info -plist "$FOUND_DRIVE" > "$WORK/drive.plist" 2>/dev/null
  mount="$(plist "$WORK/drive.plist" MountPoint)"
  [ -n "$mount" ] || stop "Couldn't open the ClaudeOS drive."
  choose_folders
  local free
  free="$(plist "$WORK/drive.plist" FreeSpace)"
  case "$free" in ''|*[!0-9]*) free=0 ;; esac
  if [ "$free" -gt 0 ] && [ "$CHOSEN_BYTES" -gt "$free" ]; then
    stop "The ClaudeOS drive has $(size_text "$free") free; those folders need $(size_text "$CHOSEN_BYTES")."
  fi
  # shellcheck disable=SC2086
  [ -n "$CHOSEN" ] && copy_folders "$mount" $CHOSEN
  good "Done. ClaudeOS sees them the next time it starts (in Mac, on the ClaudeOS drive)."
}

sign_in_again() {
  local mount
  find_install "$(system_disk)" || true
  [ -n "$FOUND_DRIVE" ] || stop "There's no ClaudeOS drive on this Mac. Install ClaudeOS first."
  diskutil mount "$FOUND_DRIVE" >> "$LOG" 2>&1
  diskutil info -plist "$FOUND_DRIVE" > "$WORK/drive.plist" 2>/dev/null
  mount="$(plist "$WORK/drive.plist" MountPoint)"
  [ -n "$mount" ] || stop "Couldn't open the ClaudeOS drive."
  sign_in
  mkdir -p "$mount/.claudeos"
  ( umask 077
    printf '%s\n' "$TOKEN" > "$mount/.claudeos/claude_token"
    [ -n "$LOGIN" ] && printf '%s\n' "$LOGIN" > "$mount/.claudeos/claude_login.json"
  )
  TOKEN="" LOGIN=""
  good "Done. Restart into ClaudeOS; it takes the new sign-in when it starts and deletes this copy."
}

choose_start() {
  step "What starts when the Mac turns on"
  find_install "$(system_disk)" || true
  [ -n "$FOUND_STUB" ] || stop "ClaudeOS isn't installed on this Mac."
  ask_choice "What should start when the Mac turns on?" \
    "ClaudeOS (macOS stays one restart away: in ClaudeOS, say \"switch to macOS\")" \
    "macOS (ClaudeOS stays installed: hold the power button at start to choose it)"
  local target
  if [ "$CHOICE" = 1 ]; then
    target="$(diskutil apfs list -plist "$FOUND_STUB" 2>/dev/null > "$WORK/stub.plist"; plist "$WORK/stub.plist" Containers.0.Volumes.0.DeviceIdentifier)"
  else
    target="$(plist "$WORK/root.plist" DeviceIdentifier)"
  fi
  [ -n "$target" ] || stop "Setup couldn't find that system's start-up volume."
  read_secret "Your Mac's password (it stays hidden):"
  if printf '%s' "$SECRET" | sudo bless --setBoot --device "/dev/$target" --user "$(id -un)" --stdinpass >> "$LOG" 2>&1; then
    SECRET=""
    good "Done."
  else
    SECRET=""
    warn "macOS didn't accept that. You can also choose in System Settings, General, Startup Disk."
    open "x-apple.systempreferences:com.apple.Startup-Disk-Settings.extension" 2>/dev/null || true
  fi
}

remove() {
  step "Remove ClaudeOS from this Mac"
  read_disk
  find_install "$DISK" || { say "ClaudeOS isn't installed on this Mac. Nothing was changed."; return 0; }
  say "This deletes ClaudeOS and gives its space back to macOS:"
  [ -n "$FOUND_STUB" ] && say "  * its start-up volume ($FOUND_STUB)"
  [ -n "$FOUND_ESP" ] && say "  * its EFI partition and its two Linux partitions ($FOUND_ESP ${FOUND_BOOT:-} ${FOUND_LINUX:-})"
  local mount="" used=0
  if [ -n "$FOUND_DRIVE" ]; then
    diskutil mount "$FOUND_DRIVE" >> "$LOG" 2>&1
    diskutil info -plist "$FOUND_DRIVE" > "$WORK/drive.plist" 2>/dev/null
    mount="$(plist "$WORK/drive.plist" MountPoint)"
    [ -n "$mount" ] && used="$(du -sk "$mount" 2>/dev/null | awk '{ printf "%d", $1 * 1024 }')"
    say "  * the ClaudeOS drive ($FOUND_DRIVE), with $(size_text "${used:-0}") of files"
  fi
  say "macOS, your apps and the files in your macOS folders stay as they are."
  if [ -n "$mount" ] && [ -n "$(find "$mount" -type f ! -path '*/.claudeos/*' ! -path '*/.Spotlight-V100/*' \
       ! -path '*/.fseventsd/*' ! -name '.metadata_never_index' ! -name '.DS_Store' 2>/dev/null | head -n 1)" ]; then
    if ask_yes "Copy what's on the ClaudeOS drive into a folder \"ClaudeOS files\" in your home folder first?" y; then
      local freebytes
      freebytes="$(df -k "$ROOT" | awk 'NR==2 { printf "%d", $4 * 1024 }')"
      if [ "$used" -ge "$freebytes" ]; then
        stop "macOS has $(size_text "$freebytes") free, not enough for the $(size_text "$used") on the ClaudeOS drive. Nothing was changed."
      fi
      say "Copying..."
      COPYFILE_DISABLE=1 rsync -rt --exclude '.claudeos' --exclude '.metadata_never_index' --exclude '.Spotlight-V100' \
        --exclude '.fseventsd' "$mount/" "$ROOT/ClaudeOS files/" >> "$LOG" 2>&1 \
        || stop "Copying the ClaudeOS drive didn't finish (details: $LOG). Nothing was removed."
      good "Copied to \"ClaudeOS files\" in your home folder."
    fi
  fi
  ask_word "Type REMOVE to remove ClaudeOS (anything else keeps it):" REMOVE || { say "Kept ClaudeOS. Nothing was changed."; return 0; }
  sudo -v || stop "Setup needs your Mac's password to remove ClaudeOS."
  # macOS first in the startup options, so the Mac starts it by itself afterwards.
  local macos_vol
  macos_vol="$(plist "$WORK/root.plist" DeviceIdentifier)"
  read_secret "Your Mac's password, to make macOS the start-up disk again (it stays hidden):"
  if ! printf '%s' "$SECRET" | sudo bless --setBoot --device "/dev/$macos_vol" --user "$(id -un)" --stdinpass >> "$LOG" 2>&1; then
    warn "macOS didn't take the password; after this, choose macOS in System Settings, General, Startup Disk."
  fi
  SECRET=""
  [ -n "$mount" ] && diskutil unmount force "$FOUND_DRIVE" >> "$LOG" 2>&1
  if [ -n "$FOUND_STUB" ]; then
    sudo diskutil apfs deleteContainer "$FOUND_STUB" >> "$LOG" 2>&1 || stop "Couldn't delete ClaudeOS's start-up volume (details: $LOG)."
  fi
  local part
  for part in $FOUND_ESP $FOUND_BOOT $FOUND_LINUX $FOUND_DRIVE; do
    sudo diskutil eraseVolume free free "$part" >> "$LOG" 2>&1 || stop "Couldn't delete $part (details: $LOG). Run setup again to finish."
  done
  good "ClaudeOS is deleted."
  if sudo diskutil apfs resizeContainer "$STORE" 0 >> "$LOG" 2>&1; then
    good "macOS has its space back."
  else
    warn "macOS's partition couldn't grow into the free space now (details: $LOG). Run setup again to try once more."
  fi
  rm -f "$ROOT/Documents/ClaudeOS recovery password.txt"
  good "Done. This Mac is back to just macOS."
}

main() {
  mkdir -p "$WORK"
  chmod 700 "$WORK"
  printf '\n\033[1mClaudeOS Setup\033[0m\n'
  say "ClaudeOS is its own system, started instead of macOS. macOS, your apps and your files stay, one restart away."
  [ "$PLAN_ONLY" = 1 ] && warn "Plan only: nothing will be changed."
  ask_choice "What would you like to do?" \
    "Install ClaudeOS on this Mac" \
    "Send more files to ClaudeOS" \
    "Sign ClaudeOS in to Claude again" \
    "Choose what starts when the Mac turns on: ClaudeOS or macOS" \
    "Remove ClaudeOS from this Mac"
  case "$CHOICE" in
    1) install ;;
    2) send_files ;;
    3) sign_in_again ;;
    4) choose_start ;;
    5) remove ;;
  esac
}

# Run as a program; tests/mac_ci/run.sh sources it instead, to try its parts one by one on a real Mac.
if [ "${BASH_SOURCE[0]}" = "$0" ]; then
  main "$@"
fi
