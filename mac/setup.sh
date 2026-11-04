#!/bin/bash
# Chat Computer Setup for Mac. Run it in Terminal:
#     bash ~/Downloads/ChatComputer/mac/setup.sh
# (or double-click "Chat Computer Setup.command" next to this folder).
#
# Chat Computer on a Mac is its own system, started instead of macOS (like on a PC, where it is started instead of
# Windows). macOS stays, with your apps and files, one restart away. It works on Macs with Apple silicon M1,
# M2 and M3 (not M3 Ultra): Chat Computer is built on Fedora Asahi Remix, and Asahi Linux doesn't support newer
# chips yet.
#
# Installing it:
#   1. checks this Mac, notes the Wi-Fi, and asks which folders to bring over: Chat Computer can't open macOS's
#      own files (on Apple silicon
#      they're always encrypted, FileVault or not, with keys in the Secure Enclave that Linux can't use yet),
#      so setup copies them now, onto a drive called ChatComp that both open;
#   2. downloads the Asahi installer and Fedora Asahi Remix's Minimal image (mac/manifest.sh: pinned, checked
#      against SHA-256 fingerprints) and runs the installer, answering its questions (mac/drive-installer.exp):
#      it makes room next to macOS and installs Fedora there; you type your Mac's password once;
#   3. makes the Chat Computer drive in the space left over and copies your folders onto it;
#   4. leaves Chat Computer's first start on Chat Computer's EFI partition (mac/esp): your account and Wi-Fi;
#   5. shows the one step Apple requires in the Mac's startup options, and shuts the Mac down.
# On its first start Chat Computer sets itself up from there and installs the rest with the internet, then downloads
# the open model it thinks with (Qwen's, chosen for this Mac's memory) and runs it on the Mac itself: no account,
# no sign-in, nothing to pay.
#
# It can also send more folders to Chat Computer, choose what starts when the Mac turns on, and remove Chat Computer
# completely. Written for the bash that comes with macOS (3.2).
# Messages and logs go to the person's own log on purpose, also from commands run with sudo; a failed check
# stops setup (stop exits), so "A && B || stop" can't fall through.
# shellcheck disable=SC2015,SC2024,SC1091
set -u

ROOT="${CHATCOMPUTER_MAC_ROOT:-$HOME}"         # tests point these at a pretend Mac
WORK="${CHATCOMPUTER_SETUP_WORK:-$ROOT/.chatcomputer-setup}"
PLAN_ONLY="${CHATCOMPUTER_PLAN_ONLY:-0}"       # 1: check and plan, change nothing
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SRC="$(cd "$HERE/.." && pwd)"                  # the folder you downloaded (it has chatcomputer/, linux/ and mac/)
LOG="$WORK/setup.log"
# shellcheck source=manifest.sh
. "${CHATCOMPUTER_MANIFEST:-$HERE/manifest.sh}"

GB=1000000000
MIN_MACOS_FREE=$((38 * GB))      # the Asahi installer always leaves macOS this much free space, for its updates
CHATCOMPUTER_MIN=$((30 * GB))    # Chat Computer's own space: Fedora, Chat Computer, its voice, and room to work
CHATCOMPUTER_WANT=$((60 * GB))
DRIVE_ROOM=$((10 * GB))          # room on the Chat Computer drive beyond what's copied there now
OS_NAME="Chat Computer"          # the name in the Mac's startup options (asahi-bless in Chat Computer uses it too)
DRIVE_NAME="ChatComp"            # the drive both systems open
FOLDERS="Desktop Documents Downloads Pictures Movies Music"

# ------------------------------------------------------------------ talking to the person
# Anything shaped like an API key (sk-ant-...) is blanked out before it is written to the log.
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
    stop "Chat Computer starts on its own only on Macs with Apple silicon (M1, M2 or M3). This Mac has an Intel processor."
  fi
  CHIP="$(sysctl -n machdep.cpu.brand_string 2>/dev/null)"
  case "$CHIP" in
    "Apple M1"*|"Apple M2"*) ;;
    "Apple M3 Ultra"*)
      stop "This Mac has an $CHIP. Chat Computer works on M1, M2 and M3 Macs, but not yet on the Mac Studio with M3 Ultra: Asahi Linux, which it's built on, doesn't support it yet." ;;
    "Apple M3"*)
      say "This Mac has an $CHIP. Asahi Linux, which Chat Computer is built on, supports M3 Macs only in part so far:"
      say "no graphics acceleration (pages and videos are slower), no brightness control and no other screens."
      say "On the iMac with M3, its speakers and microphones don't work yet either; AirPods do." ;;
    *)
      [ -n "$CHIP" ] || CHIP="Apple chip setup doesn't know"
      stop "This Mac has an $CHIP. Chat Computer can start on its own only on Macs with M1, M2 or M3: Asahi Linux, which it's built on, doesn't support newer chips yet." ;;
  esac
  if [ "$(sysctl -n kern.hv_vmm_present 2>/dev/null)" = 1 ] && [ "$PLAN_ONLY" != 1 ]; then
    stop "This is a virtual Mac. Chat Computer installs on a real Mac's own disk."
  fi
  MACOS="$(sw_vers -productVersion 2>/dev/null)"
  local major minor
  major="${MACOS%%.*}"
  minor="${MACOS#*.}"; minor="${minor%%.*}"
  case "$major$minor" in ''|*[!0-9]*) stop "This doesn't look like a Mac (no macOS version found)." ;; esac
  if [ "$major" -lt 13 ] || { [ "$major" -eq 13 ] && [ "$minor" -lt 5 ]; }; then
    stop "Chat Computer needs macOS 13.5 or newer to install; this Mac has macOS $MACOS. Update it in System Settings, then run setup again."
  fi
  if ! id -Gn 2>/dev/null | tr ' ' '\n' | grep -qx admin; then
    stop "Your account on this Mac isn't an administrator. Log in as one, then run setup again."
  fi
  say "macOS $MACOS on an $CHIP."
  # Asahi Linux can't use screens connected over USB-C or Thunderbolt yet, only the HDMI port.
  case "$(system_profiler SPHardwareDataType 2>/dev/null | sed -n 's/^ *Model Name: //p')" in
    "Mac mini"*|"Mac Studio"*|"Mac Pro"*)
      say "In Chat Computer, the screen must be connected to this Mac's HDMI port: screens on USB-C or Thunderbolt"
      say "don't work in Asahi Linux yet." ;;
  esac
}

# APFS containers, from the list of all of them: diskutil describes a container by its own name (disk3), not by
# the partition it's on (disk0s2), and the partition list doesn't say what's inside.
read_apfs() { diskutil apfs list -plist > "$WORK/apfs.plist" 2>/dev/null; }
container_on() {   # container_on PARTITION: the number of the APFS container on it (in apfs.plist), or fails
  local c=0 ref store
  while ref="$(plist "$WORK/apfs.plist" "Containers.$c.ContainerReference")" && [ -n "$ref" ]; do
    store="$(plist "$WORK/apfs.plist" "Containers.$c.DesignatedPhysicalStore")"
    [ -n "$store" ] || store="$(plist "$WORK/apfs.plist" "Containers.$c.PhysicalStores.0.DeviceIdentifier")"
    if [ "$store" = "$1" ]; then printf '%s' "$c"; return 0; fi
    c=$((c + 1))
  done
  return 1
}
volume_names() {   # volume_names CONTAINER: its volumes' names, one per line
  local v=0 name
  while name="$(plist "$WORK/apfs.plist" "Containers.$1.Volumes.$v.Name")" && [ -n "$name" ]; do
    printf '%s\n' "$name"
    v=$((v + 1))
  done
}
system_volume() {   # system_volume CONTAINER: its system volume (the one bless takes), else its first volume
  local v=0 id first=""
  while id="$(plist "$WORK/apfs.plist" "Containers.$1.Volumes.$v.DeviceIdentifier")" && [ -n "$id" ]; do
    [ -n "$first" ] || first="$id"
    if [ "$(plist "$WORK/apfs.plist" "Containers.$1.Volumes.$v.Roles.0")" = System ]; then printf '%s' "$id"; return 0; fi
    v=$((v + 1))
  done
  printf '%s' "$first"
  [ -n "$first" ]
}
macos_volume() {   # macOS's system volume, like disk3s1 (/ is mounted from a snapshot of it, which bless doesn't take)
  local store c
  diskutil info -plist / > "$WORK/root.plist" 2>/dev/null || return 1
  store="$(plist "$WORK/root.plist" APFSPhysicalStores.0.APFSPhysicalStore)"
  read_apfs
  c="$(container_on "$store")" || return 1
  system_volume "$c"
}

# Chat Computer's pieces on this Mac, from diskutil: its start-up volume's container (with a volume "Chat Computer"), its
# EFI partition ("EFI - CHAT", the Asahi installer's name for it), the two Linux partitions after it, and the
# Chat Computer drive.
find_install() {
  FOUND_STUB="" FOUND_STUB_VOLUME="" FOUND_ESP="" FOUND_BOOT="" FOUND_LINUX="" FOUND_DRIVE=""
  local d="$1" i=0 id content name after="" c
  diskutil list -plist "$d" > "$WORK/parts.plist" 2>/dev/null || return 1
  read_apfs
  while id="$(plist "$WORK/parts.plist" "AllDisksAndPartitions.0.Partitions.$i.DeviceIdentifier")" && [ -n "$id" ]; do
    content="$(plist "$WORK/parts.plist" "AllDisksAndPartitions.0.Partitions.$i.Content")"
    name="$(plist "$WORK/parts.plist" "AllDisksAndPartitions.0.Partitions.$i.VolumeName")"
    case "$content" in
      Apple_APFS)
        if c="$(container_on "$id")" && volume_names "$c" | grep -qx "$OS_NAME"; then
          FOUND_STUB="$id"
          FOUND_STUB_VOLUME="$(system_volume "$c")"
        fi
        after="" ;;
      EFI)
        case "$(printf '%s' "$name" | tr '[:lower:]' '[:upper:]')" in "EFI - CHAT"*) FOUND_ESP="$id"; after=esp ;; *) after="" ;; esac ;;
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
  say "Chat Computer can't open macOS's own files: on this Mac they're always encrypted, with keys only macOS can use"
  say "(FileVault or not). So setup copies the folders you choose onto a drive called ChatComp that both systems open;"
  say "macOS keeps its own copies. What you save on that drive in Chat Computer is in macOS too, and you can send more"
  say "later (run setup again)."
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
  if ask_yes "Bring all of these to Chat Computer ($(size_text "$total"))?" y; then
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

# ------------------------------------------------------------------ Wi-Fi for Chat Computer's first start
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
  step "Wi-Fi for Chat Computer"
  say "Chat Computer finishes installing itself the first time it starts, from the internet, so it needs your Wi-Fi."
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
  [ -n "$WIFI_SSID" ] || stop "Chat Computer needs a Wi-Fi network for its first start."
  log "Wi-Fi: $WIFI_SSID"
  read_secret "Password for \"$WIFI_SSID\" (it stays hidden; Enter = look it up in this Mac's keychain, which may ask for your Mac's password):"
  WIFI_PSK="$SECRET"
  SECRET=""
  if [ -z "$WIFI_PSK" ]; then
    WIFI_PSK="$(security find-generic-password -D 'AirPort network password' -a "$WIFI_SSID" -w 2>/dev/null)"
    if [ -n "$WIFI_PSK" ]; then
      good "Found it in the keychain."
    elif ! ask_yes "No password found. Is \"$WIFI_SSID\" an open network (no password)?" n; then
      stop "Chat Computer needs the Wi-Fi password for its first start. Run setup again and type it."
    fi
  fi
}

# ------------------------------------------------------------------ the space
plan_space() {
  step "Where Chat Computer goes"
  read_disk
  DRIVE_BYTES=$((CHOSEN_BYTES + CHOSEN_BYTES / 10 + DRIVE_ROOM))
  CHATCOMPUTER_BYTES="$CHATCOMPUTER_WANT"
  if [ $((CHATCOMPUTER_BYTES + DRIVE_BYTES)) -gt "$AVAIL" ]; then
    CHATCOMPUTER_BYTES=$((AVAIL - DRIVE_BYTES))
  fi
  if [ "$CHATCOMPUTER_BYTES" -lt "$CHATCOMPUTER_MIN" ]; then
    say "macOS can give Chat Computer $(size_text "$AVAIL") and still keep 38 GB free for its updates."
    if [ "$AVAIL" -ge $((CHATCOMPUTER_MIN + DRIVE_ROOM)) ] && [ "$CHOSEN_BYTES" -gt 0 ]; then
      stop "That's not enough for Chat Computer ($(size_text "$CHATCOMPUTER_MIN")) and the $(size_text "$CHOSEN_BYTES") of folders. Run setup again and bring fewer folders, or free up space in macOS."
    fi
    stop "Chat Computer needs $(size_text $((CHATCOMPUTER_MIN + DRIVE_ROOM))). Free up space in macOS, then run setup again."
  fi
  NEED=$((CHATCOMPUTER_BYTES + DRIVE_BYTES))
  MACOS_NEW=$((TOTAL - NEED))
  say "macOS's partition is $(size_text "$TOTAL") with $(size_text "$FREE") free."
  say "Chat Computer gets $(size_text "$CHATCOMPUTER_BYTES"), and the Chat Computer drive $(size_text "$DRIVE_BYTES")."
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
  # Fedora's image. Chat Computer's icon for the Mac's startup options goes into its package (the installer takes it
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
       "$py" - "$pkg" "$HERE/chatcomputer.icns" <<'EOF'
import sys, zipfile
with zipfile.ZipFile(sys.argv[1], "a") as z:
    if "chatcomputer.icns" not in z.namelist():
        z.write(sys.argv[2], "chatcomputer.icns")
EOF
  then
    shasum -a 256 "$pkg" | awk '{print $1}' > "$pkg.with-icon"
  else
    # Without it in the package, the installer must not look for it: it shows its own icon instead.
    if [ "$(shasum -a 256 "$pkg" | awk '{print $1}')" != "$FEDORA_PACKAGE_SHA256" ]; then
      rm -f "$pkg"
      stop "Fedora's package was damaged while Chat Computer's icon was added to it. Run setup again (it downloads it again)."
    fi
    warn "Chat Computer's icon couldn't be added; the startup options show the installer's own."
    sed '/"icon":/d' "$HERE/installer_data.json" > "$INST/installer_data.json"
  fi
}

run_installer() {
  step "Installing Chat Computer next to macOS"
  say "The Asahi installer makes room next to macOS and installs Chat Computer's base (Fedora Linux) there. Setup"
  say "answers its questions. When it asks for your Mac's password, type it (it stays hidden) and press Enter."
  say "It takes about 10 to 20 minutes; your Mac may seem to freeze while macOS's partition shrinks. That's normal."
  say "First, your Mac's password for setup itself:"
  sudo -v || stop "Setup needs your Mac's password to install Chat Computer."
  local code=0
  (
    cd "$INST" || exit 6
    export REPO_BASE="$WORK" DISTRO="Chat Computer" DISTRO_DOCS="https://docs.fedoraproject.org/en-US/fedora-asahi-remix/"
    unset REPORT REPORT_TAG EXPERT
    # shellcheck disable=SC2086
    CHATCOMPUTER_MACOS_SIZE="${MACOS_NEW}B" CHATCOMPUTER_OS_SIZE="${CHATCOMPUTER_BYTES}B" \
      ${CHATCOMPUTER_EXPECT:-expect} -f "$HERE/drive-installer.exp" -- ${CHATCOMPUTER_INSTALLER_CMD:-caffeinate -dis sudo -E ./install.sh}
  ) || code=$?
  [ -f "$INST/installer.log" ] && cp "$INST/installer.log" "$WORK/asahi-installer.log"
  case "$code" in
    0) ;;
    2) stop "macOS's partition couldn't be made smaller. If Time Machine keeps local snapshots, see https://alx.sh/tmcleanup; then run setup again." ;;
    3) stop "There wasn't enough free space after all. Free up space in macOS, then run setup again." ;;
    5) stop "macOS needs updating first. Update it in System Settings, then run setup again." ;;
    *) stop "The Asahi installer didn't finish (see above; its log is $WORK/asahi-installer.log). If it made room or partitions, run setup again and choose \"Remove Chat Computer from this Mac\" to clean up." ;;
  esac
  # Which partitions it made, from its log: "Installing partition {'name': 'EFI', ...} -> disk0s4".
  EFI_DEV="$(sed -n "s/.*Installing partition {'name': 'EFI'.* -> \(disk[0-9]*s[0-9]*\)\$/\1/p" "$WORK/asahi-installer.log" | tail -n 1)"
  ROOT_DEV="$(sed -n "s/.*Installing partition {'name': 'Root'.* -> \(disk[0-9]*s[0-9]*\)\$/\1/p" "$WORK/asahi-installer.log" | tail -n 1)"
  [ -n "$EFI_DEV" ] && [ -n "$ROOT_DEV" ] || stop "Setup couldn't find the partitions the installer made (its log: $WORK/asahi-installer.log)."
  good "Chat Computer's base is installed."
}

# ------------------------------------------------------------------ the Chat Computer drive
make_drive() {
  step "The Chat Computer drive"
  sudo diskutil addPartition "$ROOT_DEV" ExFAT "$DRIVE_NAME" 0 >> "$LOG" 2>&1 \
    || stop "Couldn't make the Chat Computer drive in the space left over (details: $LOG)."
  find_install "$DISK" || true
  [ -n "$FOUND_DRIVE" ] || stop "Setup made the Chat Computer drive but can't find it now."
  diskutil info -plist "$FOUND_DRIVE" > "$WORK/drive.plist" 2>/dev/null
  DRIVE_PARTUUID="$(plist "$WORK/drive.plist" DiskUUID)"
  DRIVE_MOUNT="$(plist "$WORK/drive.plist" MountPoint)"
  if [ -z "$DRIVE_MOUNT" ]; then
    diskutil mount "$FOUND_DRIVE" >> "$LOG" 2>&1
    diskutil info -plist "$FOUND_DRIVE" > "$WORK/drive.plist" 2>/dev/null
    DRIVE_MOUNT="$(plist "$WORK/drive.plist" MountPoint)"
  fi
  [ -n "$DRIVE_MOUNT" ] && [ -d "$DRIVE_MOUNT" ] || stop "Couldn't open the Chat Computer drive."
  # Spotlight and Finder's own files would only clutter it.
  sudo mdutil -i off "$DRIVE_MOUNT" >> "$LOG" 2>&1 || true
  touch "$DRIVE_MOUNT/.metadata_never_index" 2>/dev/null
  mkdir -p "$DRIVE_MOUNT/.chatcomputer"
  good "The Chat Computer drive is ready ($(size_text "$DRIVE_BYTES"))."
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

# ------------------------------------------------------------------ Chat Computer's first start
write_first_start() {
  step "Preparing Chat Computer's first start"
  local esp_mount p
  diskutil mount "$EFI_DEV" >> "$LOG" 2>&1
  diskutil info -plist "$EFI_DEV" > "$WORK/esp.plist" 2>/dev/null
  esp_mount="$(plist "$WORK/esp.plist" MountPoint)"
  [ -n "$esp_mount" ] && [ -f "$esp_mount/EFI/fedora/grub.cfg" ] || stop "Couldn't open Chat Computer's EFI partition ($EFI_DEV)."
  p="$WORK/payload/chatcomputer"
  rm -rf "$WORK/payload"
  mkdir -p "$p/secrets"
  chmod 700 "$WORK/payload" "$p/secrets"
  # Chat Computer itself: what runs on Linux (no setup tools for Windows or macOS).
  # (No extended attributes: macOS adds its own to files, which Fedora's tar would only warn about.)
  COPYFILE_DISABLE=1 tar --no-xattrs --exclude '__pycache__' --exclude '.DS_Store' -czf "$p/app.tar.gz" -C "$SRC" \
    chatcomputer linux requirements-linux.txt pyproject.toml README.md ARCHITECTURE.md 2>> "$LOG" \
    || stop "Couldn't pack Chat Computer."
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
  )
  printf '{\n  "user_name": "%s",\n  "timezone": "%s",\n  "locale": "en_US.UTF-8"\n}\n' \
    "$(printf '%s' "$REAL_NAME" | sed 's/\\/\\\\/g; s/"/\\"/g')" "$TZONE" > "$p/settings.json"
  sudo mkdir -p "$esp_mount/chatcomputer" || stop "Couldn't write to Chat Computer's EFI partition."
  sudo cp "$esp_mount/EFI/fedora/grub.cfg" "$esp_mount/chatcomputer/fedora-grub.cfg"
  sudo cp -R "$p/." "$esp_mount/chatcomputer/" || stop "Couldn't write to Chat Computer's EFI partition."
  sed -e "s/@KERNEL@/$FEDORA_KERNEL/g" -e "s/@BOOT_UUID@/$FEDORA_BOOT_UUID/g" -e "s/@ROOT_UUID@/$FEDORA_ROOT_UUID/g" \
    "$HERE/esp/grub-firststart.cfg" > "$WORK/grub-firststart.cfg"
  sudo cp "$WORK/grub-firststart.cfg" "$esp_mount/EFI/fedora/grub.cfg" || stop "Couldn't write to Chat Computer's EFI partition."
  sync
  rm -rf "$WORK/payload"
  diskutil unmount "$EFI_DEV" >> "$LOG" 2>&1
  good "Chat Computer will set itself up when it first starts."
}

# ------------------------------------------------------------------ the four things setup does
install() {
  check_mac
  if find_install "$(system_disk)"; then
    stop "Chat Computer is already on this Mac. To install it again, run setup and choose \"Remove Chat Computer from this Mac\" first."
  fi
  if ! curl -sSfI --max-time 20 "https://fedora-asahi-remix.org/" -o /dev/null 2>/dev/null; then
    stop "This Mac can't reach the internet (fedora-asahi-remix.org). Connect to Wi-Fi, then run setup again."
  fi
  choose_folders
  plan_space
  REAL_NAME="$(id -F 2>/dev/null)"
  [ -n "$REAL_NAME" ] || REAL_NAME="$(id -un)"
  ACCOUNT="$(id -un | tr '[:upper:]' '[:lower:]' | tr -c 'a-z0-9_-' '-' | sed 's/^[^a-z_]*//; s/-*$//' | cut -c1-30)"
  [ -n "$ACCOUNT" ] || ACCOUNT="chatcomputer"
  TZONE="$(readlink /etc/localtime 2>/dev/null | sed -n 's|.*zoneinfo/||p')"
  [ -n "$TZONE" ] || TZONE="UTC"
  say "Name: $REAL_NAME   Account: $ACCOUNT   Time zone: $TZONE"
  if [ "$PLAN_ONLY" = 1 ]; then
    step "Plan only"
    say "Nothing was changed. setup would shrink macOS's partition ($STORE) to $(size_text "$MACOS_NEW"),"
    say "give Chat Computer $(size_text "$CHATCOMPUTER_BYTES") and the Chat Computer drive $(size_text "$DRIVE_BYTES"), and copy:${CHOSEN:- nothing}."
    printf 'PLAN store=%s macos_new=%s chatcomputer=%s drive=%s folders=%s\n' "$STORE" "$MACOS_NEW" "$CHATCOMPUTER_BYTES" "$DRIVE_BYTES" "$CHOSEN" >> "$LOG"
    return 0
  fi
  ask_wifi
  RECOVERY="$(LC_ALL=C tr -dc 'a-km-z2-9' < /dev/urandom | head -c 16 | sed 's/\(....\)/\1-/g; s/-$//')"
  INSTALL_ID="$(od -An -N12 -tx1 /dev/urandom | tr -d ' \n')"

  step "Ready"
  say "This is what happens now:"
  say "  1. macOS's partition shrinks to $(size_text "$MACOS_NEW"). Your apps and files stay."
  say "  2. Chat Computer gets $(size_text "$CHATCOMPUTER_BYTES") next to macOS (its base is Fedora Linux, downloaded now: 1.2 GB)."
  say "  3. The Chat Computer drive gets $(size_text "$DRIVE_BYTES"), with copies of:${CHOSEN:- (nothing yet)}."
  say "  4. Then the Mac shuts down, and you do one short step in its startup options. Setup shows you how."
  say "Chat Computer then starts instead of macOS. macOS stays one restart away: in Chat Computer, say \"switch to macOS\"."
  ask_yes "Go ahead?" n || stop "Stopped at your request. Nothing was changed."

  prepare_installer
  run_installer
  make_drive
  # shellcheck disable=SC2086
  [ -n "$CHOSEN" ] && copy_folders "$DRIVE_MOUNT" $CHOSEN
  write_first_start
  WIFI_PSK=""
  local note="$ROOT/Documents/Chat Computer recovery password.txt"
  mkdir -p "$ROOT/Documents"
  printf 'Chat Computer recovery password\n\nAccount: %s\nPassword: %s\n\nYou normally never need this: Chat Computer starts without a password. It is for repairs only.\n' \
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
  say "  5. A window \"Chat Computer installer (second step)\" opens. Press Enter, then type your Mac's user name and"
  say "     password when asked (it asks twice). The note about the security level is about Chat Computer only:"
  say "     macOS keeps its own."
  say "  6. At \"Installation complete! Press enter to reboot.\", press Enter."
  say ""
  say "Chat Computer then sets itself up from your Wi-Fi (about 20 to 40 minutes), restarts once, and says hello."
  say "If the Mac starts macOS instead, start again from step 1."
  if ask_yes "Shut the Mac down now?" y; then
    sudo shutdown -h now
  else
    say "When you're ready, shut the Mac down from the Apple menu and follow the steps above."
  fi
}

send_files() {
  step "Send files to Chat Computer"
  local mount
  find_install "$(system_disk)" || true
  [ -n "$FOUND_DRIVE" ] || stop "There's no Chat Computer drive on this Mac. Install Chat Computer first."
  diskutil mount "$FOUND_DRIVE" >> "$LOG" 2>&1
  diskutil info -plist "$FOUND_DRIVE" > "$WORK/drive.plist" 2>/dev/null
  mount="$(plist "$WORK/drive.plist" MountPoint)"
  [ -n "$mount" ] || stop "Couldn't open the Chat Computer drive."
  choose_folders
  local free
  free="$(plist "$WORK/drive.plist" FreeSpace)"
  case "$free" in ''|*[!0-9]*) free=0 ;; esac
  if [ "$free" -gt 0 ] && [ "$CHOSEN_BYTES" -gt "$free" ]; then
    stop "The Chat Computer drive has $(size_text "$free") free; those folders need $(size_text "$CHOSEN_BYTES")."
  fi
  # shellcheck disable=SC2086
  [ -n "$CHOSEN" ] && copy_folders "$mount" $CHOSEN
  good "Done. Chat Computer sees them the next time it starts (in Mac, on the Chat Computer drive)."
}

choose_start() {
  step "What starts when the Mac turns on"
  find_install "$(system_disk)" || true
  [ -n "$FOUND_STUB" ] || stop "Chat Computer isn't installed on this Mac."
  ask_choice "What should start when the Mac turns on?" \
    "Chat Computer (macOS stays one restart away: in Chat Computer, say \"switch to macOS\")" \
    "macOS (Chat Computer stays installed: hold the power button at start to choose it)"
  local target
  if [ "$CHOICE" = 1 ]; then
    target="$FOUND_STUB_VOLUME"
  else
    target="$(macos_volume)"
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
  step "Remove Chat Computer from this Mac"
  read_disk
  if ! find_install "$DISK"; then
    grow_macos || say "Chat Computer isn't installed on this Mac. Nothing was changed."
    return 0
  fi
  say "This deletes Chat Computer and gives its space back to macOS:"
  [ -n "$FOUND_STUB" ] && say "  * its start-up volume ($FOUND_STUB)"
  [ -n "$FOUND_ESP" ] && say "  * its EFI partition and its two Linux partitions ($FOUND_ESP ${FOUND_BOOT:-} ${FOUND_LINUX:-})"
  local mount="" used=0
  if [ -n "$FOUND_DRIVE" ]; then
    diskutil mount "$FOUND_DRIVE" >> "$LOG" 2>&1
    diskutil info -plist "$FOUND_DRIVE" > "$WORK/drive.plist" 2>/dev/null
    mount="$(plist "$WORK/drive.plist" MountPoint)"
    [ -n "$mount" ] && used="$(du -sk "$mount" 2>/dev/null | awk '{ printf "%d", $1 * 1024 }')"
    say "  * the Chat Computer drive ($FOUND_DRIVE), with $(size_text "${used:-0}") of files"
  fi
  say "macOS, your apps and the files in your macOS folders stay as they are."
  if [ -n "$mount" ] && [ -n "$(find "$mount" -type f ! -path '*/.chatcomputer/*' ! -path '*/.Spotlight-V100/*' \
       ! -path '*/.fseventsd/*' ! -name '.metadata_never_index' ! -name '.DS_Store' 2>/dev/null | head -n 1)" ]; then
    if ask_yes "Copy what's on the Chat Computer drive into a folder \"Chat Computer files\" in your home folder first?" y; then
      local freebytes
      freebytes="$(df -k "$ROOT" | awk 'NR==2 { printf "%d", $4 * 1024 }')"
      if [ "$used" -ge "$freebytes" ]; then
        stop "macOS has $(size_text "$freebytes") free, not enough for the $(size_text "$used") on the Chat Computer drive. Nothing was changed."
      fi
      say "Copying..."
      COPYFILE_DISABLE=1 rsync -rt --exclude '.chatcomputer' --exclude '.metadata_never_index' --exclude '.Spotlight-V100' \
        --exclude '.fseventsd' "$mount/" "$ROOT/Chat Computer files/" >> "$LOG" 2>&1 \
        || stop "Copying the Chat Computer drive didn't finish (details: $LOG). Nothing was removed."
      good "Copied to \"Chat Computer files\" in your home folder."
    fi
  fi
  ask_word "Type REMOVE to remove Chat Computer (anything else keeps it):" REMOVE || { say "Kept Chat Computer. Nothing was changed."; return 0; }
  sudo -v || stop "Setup needs your Mac's password to remove Chat Computer."
  # macOS first in the startup options, so the Mac starts it by itself afterwards.
  local macos_vol
  macos_vol="$(macos_volume)"
  if [ -n "$macos_vol" ]; then
    read_secret "Your Mac's password, to make macOS the start-up disk again (it stays hidden):"
    if ! printf '%s' "$SECRET" | sudo bless --setBoot --device "/dev/$macos_vol" --user "$(id -un)" --stdinpass >> "$LOG" 2>&1; then
      warn "macOS didn't take the password; after this, choose macOS in System Settings, General, Startup Disk."
    fi
    SECRET=""
  else
    warn "Setup couldn't find macOS's start-up volume; after this, choose macOS in System Settings, General, Startup Disk."
  fi
  [ -n "$mount" ] && diskutil unmount force "$FOUND_DRIVE" >> "$LOG" 2>&1
  if [ -n "$FOUND_STUB" ]; then
    sudo diskutil apfs deleteContainer "$FOUND_STUB" >> "$LOG" 2>&1 || stop "Couldn't delete Chat Computer's start-up volume (details: $LOG)."
  fi
  local part
  for part in $FOUND_ESP $FOUND_BOOT $FOUND_LINUX $FOUND_DRIVE; do
    sudo diskutil eraseVolume free free "$part" >> "$LOG" 2>&1 || stop "Couldn't delete $part (details: $LOG). Run setup again to finish."
  done
  good "Chat Computer is deleted."
  rm -f "$ROOT/Documents/Chat Computer recovery password.txt"
  if sudo diskutil apfs resizeContainer "$STORE" 0 >> "$LOG" 2>&1; then
    good "macOS has its space back. This Mac is back to just macOS."
  else
    warn "macOS's partition couldn't grow into the free space now (details: $LOG). To try again, run setup and"
    warn "choose \"Remove Chat Computer from this Mac\" once more."
  fi
}

# After a removal whose last step didn't work: free space right after macOS's partition goes back to macOS.
grow_macos() {
  local cur max
  diskutil apfs resizeContainer "$STORE" limits -plist > "$WORK/limits.plist" 2>/dev/null || return 1
  cur="$(plist "$WORK/limits.plist" CurrentSize)"
  max="$(plist "$WORK/limits.plist" MaximumSize)"
  case "$cur$max" in ''|*[!0-9]*) return 1 ;; esac
  [ $((max - cur)) -gt "$GB" ] || return 1
  say "Chat Computer isn't on this Mac, but $(size_text $((max - cur))) of free space is right after macOS's partition."
  ask_yes "Give that space back to macOS?" y || { say "Nothing was changed."; return 0; }
  sudo -v || stop "Setup needs your Mac's password to give macOS its space."
  if sudo diskutil apfs resizeContainer "$STORE" 0 >> "$LOG" 2>&1; then
    good "macOS has its space back."
  else
    warn "macOS's partition couldn't grow into the free space (details: $LOG)."
  fi
}

main() {
  mkdir -p "$WORK"
  chmod 700 "$WORK"
  printf '\n\033[1mChat Computer Setup\033[0m\n'
  say "Chat Computer is its own system, started instead of macOS. macOS, your apps and your files stay, one restart away."
  [ "$PLAN_ONLY" = 1 ] && warn "Plan only: nothing will be changed."
  ask_choice "What would you like to do?" \
    "Install Chat Computer on this Mac" \
    "Send more files to Chat Computer" \
    "Choose what starts when the Mac turns on: Chat Computer or macOS" \
    "Remove Chat Computer from this Mac"
  case "$CHOICE" in
    1) install ;;
    2) send_files ;;
    3) choose_start ;;
    4) remove ;;
  esac
}

# Run as a program; tests/mac_ci/run.sh sources it instead, to try its parts one by one on a real Mac.
if [ "${BASH_SOURCE[0]}" = "$0" ]; then
  main "$@"
fi
