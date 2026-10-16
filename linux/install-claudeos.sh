#!/bin/bash
# install-claudeos.sh — turn a fresh Linux into the ClaudeOS "thin Linux base": boot straight into one
# full-screen voice session, with no desktop, display manager, login prompt, browser or SSH server.
# On a PC it's Ubuntu Server 24.04, next to Windows; on an Apple silicon Mac it's Fedora Asahi Remix 44
# (Minimal), next to macOS. The few differences are marked "Fedora" / "Ubuntu" below.
#
# Idempotent: safe to run repeatedly. Normally invoked by the autoinstaller:
#   curtin in-target --target=/target -- \
#       bash /opt/claudeos/app/linux/install-claudeos.sh --user <user> --from-installer
# and re-runnable by hand later for repair/upgrade.
#
# Flags:
#   --user NAME        (required) the ClaudeOS user (created by subiquity's
#                      identity section during install).
#   --from-installer   running inside the target chroot during install: only
#                      `systemctl enable` (never start), no interactive prompts,
#                      do not assume a running systemd/udev.
#   --dry-run          print the actions that would run; change nothing.
#   --skip-models      do not download voice models (done later, as the user).
#   --distro NAME      ubuntu or fedora (default: from /etc/os-release).
#
# Exit status: 0 once everything is in place (then /var/lib/claudeos/base-installed exists);
# 1 when a step ClaudeOS can't work without failed, usually for lack of internet. Run it again.
#
# Everything this script writes comes from the repo it lives in
# (/opt/claudeos/app/linux/...) — see system/ for the static config tree.
set -u
set -o pipefail

# ---------------------------------------------------------------------------
# Argument parsing.
# ---------------------------------------------------------------------------
USER_NAME=""
FROM_INSTALLER=0
DRY_RUN=0
SKIP_MODELS=0
DISTRO=""

usage() {
  cat >&2 <<EOF
Usage: install-claudeos.sh --user NAME [--from-installer] [--dry-run] [--skip-models] [--distro ubuntu|fedora]
EOF
  exit 2
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --user)            USER_NAME="${2:-}"; shift 2 || usage ;;
    --user=*)          USER_NAME="${1#*=}"; shift ;;
    --from-installer)  FROM_INSTALLER=1; shift ;;
    --dry-run)         DRY_RUN=1; shift ;;
    --skip-models)     SKIP_MODELS=1; shift ;;
    --distro)          DISTRO="${2:-}"; shift 2 || usage ;;
    -h|--help)         usage ;;
    *) echo "unknown argument: $1" >&2; usage ;;
  esac
done

[ -n "$USER_NAME" ] || { echo "error: --user is required" >&2; usage; }
# Validate the username: Debian useradd NAME_REGEX, lowercase start.
if ! [[ "$USER_NAME" =~ ^[a-z_][a-z0-9_-]*$ ]]; then
  echo "error: invalid --user name: $USER_NAME" >&2
  exit 2
fi

# Which Linux this is: Ubuntu (a PC) or Fedora (an Apple silicon Mac).
if [ -z "$DISTRO" ]; then
  # shellcheck disable=SC1091
  case "$( (. /etc/os-release 2>/dev/null && printf '%s' "${ID:-}") )" in
    fedora) DISTRO=fedora ;;
    *) DISTRO=ubuntu ;;
  esac
fi
case "$DISTRO" in ubuntu|fedora) ;; *) echo "error: --distro must be ubuntu or fedora" >&2; exit 2 ;; esac

# ---------------------------------------------------------------------------
# Paths & constants.
# ---------------------------------------------------------------------------
# Resolve the repo root from this script's own location, so the installer works
# whether it lives at /opt/claudeos/app or in a test checkout.
SELF="$(readlink -f "$0")"
LINUX_DIR="$(dirname "$SELF")"           # .../app/linux
APP_DIR="$(dirname "$LINUX_DIR")"        # .../app
SYSTEM_DIR="$LINUX_DIR/system"

OPT_APP=/opt/claudeos/app
VENV_DIR=/opt/claudeos/venv
REQ_FILE="$OPT_APP/requirements-linux.txt"
LIBEXEC_DIR=/usr/local/libexec/claudeos
STAMP=/var/lib/claudeos/base-installed   # written when every step has worked
USER_HOME="/home/$USER_NAME"
CLAUDEOS_HOME="$USER_HOME/ClaudeOS"

log()  { printf '[claudeos-install] %s\n' "$*" >&2; }
warn() { printf '[claudeos-install] WARNING: %s\n' "$*" >&2; }

# needed — a step ClaudeOS can't work without has failed (usually no internet). Stop here, before
# the later steps change anything that depends on it (for example handing the network over to
# NetworkManager when NetworkManager isn't installed). Exit status 1 and no "installed" mark, so
# the installer's finish.sh or claudeos-finish-install.service (linux/firstboot) run this again.
needed() {
  warn "$*"
  log "ClaudeOS base install is NOT complete. It is safe to run this again once the problem"
  log "(usually the internet connection) is fixed."
  exit 1
}

# run — execute a command, or just print it under --dry-run. Use for every
# state-changing action so --dry-run is honoured uniformly.
run() {
  if [ "$DRY_RUN" -eq 1 ]; then
    printf '[dry-run] %s\n' "$*" >&2
    return 0
  fi
  "$@"
}

# run_sh — like run, but for a shell pipeline given as a single string. Only
# used with STATIC strings (never user input).
run_sh() {
  if [ "$DRY_RUN" -eq 1 ]; then
    printf '[dry-run] sh -c %q\n' "$1" >&2
    return 0
  fi
  sh -c "$1"
}

# retry — run a command up to 3 times with a short pause (network operations).
retry() {
  local n=0 max=3 rc=0
  while : ; do
    if [ "$DRY_RUN" -eq 1 ]; then printf '[dry-run] %s\n' "$*" >&2; return 0; fi
    "$@" && return 0
    rc=$?
    n=$((n+1))
    if [ "$n" -ge "$max" ]; then
      warn "command failed after ${max} attempts (rc=$rc): $*"
      return "$rc"
    fi
    log "retry $n/$max after failure (rc=$rc): $*"
    sleep $((n*3))
  done
}

# install_file SRC DEST MODE [OWNER] — copy a repo file into place, with mode and
# optional owner, only rewriting when content differs (keeps it idempotent and
# avoids needless mtime churn). Also used to render templated files via a
# pre-filter (see install_templated).
install_file() {
  local src=$1 dest=$2 mode=$3 owner=${4:-}
  [ -f "$src" ] || { warn "missing source file: $src"; return 1; }
  if [ "$DRY_RUN" -eq 1 ]; then
    printf '[dry-run] install %s -> %s (mode %s%s)\n' "$src" "$dest" "$mode" \
      "${owner:+, owner $owner}" >&2
    return 0
  fi
  install -d -m 0755 "$(dirname "$dest")"
  if [ -f "$dest" ] && cmp -s "$src" "$dest"; then
    : # unchanged
  else
    install -m "$mode" "$src" "$dest"
  fi
  chmod "$mode" "$dest"
  [ -n "$owner" ] && chown "$owner" "$dest"
  return 0
}

# install_templated SRC DEST MODE [OWNER] — like install_file but replaces the
# @CLAUDEOS_USER@ token with the real username while copying.
install_templated() {
  local src=$1 dest=$2 mode=$3 owner=${4:-}
  [ -f "$src" ] || { warn "missing source file: $src"; return 1; }
  if [ "$DRY_RUN" -eq 1 ]; then
    printf '[dry-run] render %s -> %s (user=%s, mode %s)\n' "$src" "$dest" "$USER_NAME" "$mode" >&2
    return 0
  fi
  install -d -m 0755 "$(dirname "$dest")"
  local tmp; tmp="$(mktemp)"
  sed "s/@CLAUDEOS_USER@/$USER_NAME/g" "$src" > "$tmp"
  if [ -f "$dest" ] && cmp -s "$tmp" "$dest"; then
    rm -f "$tmp"
  else
    install -m "$mode" "$tmp" "$dest"
    rm -f "$tmp"
  fi
  chmod "$mode" "$dest"
  [ -n "$owner" ] && chown "$owner" "$dest"
  return 0
}

# systemctl wrapper: in a chroot (no running systemd) we may only `enable`
# (which just manages symlinks). Never `start`/`restart` from the installer
# when --from-installer; outside the installer, enable --now is fine.
#
# `systemctl enable` normally works offline (it only manages symlinks), but in a
# minimal chroot without a D-Bus/systemd it can silently no-op for a unit that
# lives in /etc/systemd/system. So after enabling we verify the wants-symlink
# exists and, if not, create it manually from the unit's own WantedBy= — this is
# exactly what `enable` would have done, and keeps us chroot-safe + idempotent.
sctl_enable() {
  local unit=$1
  if [ "$DRY_RUN" -eq 1 ]; then
    # (Printed here: the calls below send systemctl's chroot noise, and so run's dry-run line, to /dev/null.)
    printf '[dry-run] systemctl enable %s\n' "$unit" >&2
    [ "$FROM_INSTALLER" -eq 1 ] || printf '[dry-run] systemctl start %s\n' "$unit" >&2
    return 0
  fi
  if [ "$FROM_INSTALLER" -eq 1 ]; then
    systemctl enable "$unit" 2>/dev/null || true
  else
    systemctl enable "$unit" 2>/dev/null || true
    systemctl start "$unit" 2>/dev/null || warn "could not start $unit now"
  fi
  ensure_wants_symlink "$unit"
}

# ensure_wants_symlink — guarantee UNIT is wired into its WantedBy target by a
# symlink under /etc/systemd/system/<target>.wants/, creating it if `enable`
# did not. Only handles plain (non-template) units we ship or know.
ensure_wants_symlink() {
  local unit=$1 unitfile="" wantedby target d
  # Locate the unit file.
  for d in /etc/systemd/system /usr/lib/systemd/system /lib/systemd/system; do
    if [ -f "$d/$unit" ]; then unitfile="$d/$unit"; break; fi
  done
  [ -n "$unitfile" ] || return 0
  # Already enabled via some .wants symlink or an Alias? Then we are done.
  if find /etc/systemd/system -name "$unit" -path '*.wants/*' 2>/dev/null | grep -q . ; then
    return 0
  fi
  # Template units (getty@tty1) and dbus-activated services (bluetooth) are
  # handled by their own packaging/aliases; skip manual wiring for them.
  case "$unit" in
    getty@*|bluetooth.service) return 0 ;;
  esac
  wantedby=$(sed -n 's/^WantedBy=//p' "$unitfile" | head -n1)
  [ -n "$wantedby" ] || return 0
  for target in $wantedby; do
    install -d -m 0755 "/etc/systemd/system/${target}.wants"
    ln -sf "$unitfile" "/etc/systemd/system/${target}.wants/$unit"
    log "wired $unit into $target (manual .wants symlink)"
  done
}

# ---------------------------------------------------------------------------
# Specialised helpers (defined before use).
# ---------------------------------------------------------------------------

# apply_bluez_main_conf — write our BlueZ settings into /etc/bluetooth/main.conf
# idempotently, because noble's BlueZ ignores main.conf.d. We append a clearly
# marked block and, on re-run, replace the block between the markers rather than
# duplicating it. We do NOT force ControllerMode=bredr (fallback only).
apply_bluez_main_conf() {
  local conf=/etc/bluetooth/main.conf
  local begin="# >>> ClaudeOS managed block >>>"
  local end="# <<< ClaudeOS managed block <<<"
  if [ "$DRY_RUN" -eq 1 ]; then
    log "[dry-run] apply ClaudeOS BlueZ settings into $conf"
    return 0
  fi
  [ -f "$conf" ] || { warn "$conf not present; skipping BlueZ tuning"; return 0; }
  local block
  block="$(cat <<'EOF'
# >>> ClaudeOS managed block >>>
# Managed by install-claudeos.sh. Do not edit between the markers.
# (noble BlueZ ignores main.conf.d, so settings live here.)
[General]
FastConnectable = true
JustWorksRepairing = always
[Policy]
AutoEnable = true
ReconnectAttempts = 7
ReconnectIntervals = 1,2,4,8,16,32,64
# <<< ClaudeOS managed block <<<
EOF
)"
  local tmp; tmp="$(mktemp)"
  if grep -qF "$begin" "$conf"; then
    # Replace the existing block.
    awk -v b="$begin" -v e="$end" -v blk="$block" '
      $0==b {inblk=1; print blk; next}
      $0==e {inblk=0; next}
      !inblk {print}
    ' "$conf" > "$tmp"
  else
    cat "$conf" > "$tmp"
    printf '\n%s\n' "$block" >> "$tmp"
  fi
  if cmp -s "$tmp" "$conf"; then rm -f "$tmp"; else install -m 0644 "$tmp" "$conf"; rm -f "$tmp"; fi
  log "applied ClaudeOS BlueZ settings to $conf"
}

# neutralise_installer_netplan — subiquity writes 00-installer-config.yaml using
# the networkd renderer. We keep the file (so link config is preserved) but set
# its renderer to NetworkManager by dropping our 99 file (already installed) and
# removing any 'renderer: networkd' line in the installer file so NM wins.
neutralise_installer_netplan() {
  local f=/etc/netplan/00-installer-config.yaml
  if [ "$DRY_RUN" -eq 1 ]; then log "[dry-run] set NM renderer in $f"; return 0; fi
  [ -f "$f" ] || return 0
  if grep -qE '^\s*renderer:\s*networkd' "$f"; then
    sed -i 's/^\(\s*\)renderer:\s*networkd/\1renderer: NetworkManager/' "$f"
    log "switched $f renderer to NetworkManager"
  fi
  chmod 0600 "$f" 2>/dev/null || true
}

# set_ufw_defaults — default deny incoming, allow outgoing. We write ufw's
# policy files directly (idempotent, chroot-safe); the ufw unit enforces them at
# boot. No inbound ports are opened (no SSH, no services).
set_ufw_defaults() {
  if [ "$DRY_RUN" -eq 1 ]; then log "[dry-run] ufw default deny incoming / allow outgoing + enable"; return 0; fi
  if command -v ufw >/dev/null 2>&1; then
    # These commands only edit /etc/ufw/* and /etc/default/ufw; they do not need
    # the firewall to be live, so they are safe in a chroot.
    ufw --force default deny incoming  >/dev/null 2>&1 || warn "ufw default deny incoming failed"
    ufw --force default allow outgoing >/dev/null 2>&1 || warn "ufw default allow outgoing failed"
    # Mark ufw enabled in its config so the unit brings it up at boot. `ufw
    # enable` tries to load rules now (needs iptables); tolerate failure here.
    ufw --force enable >/dev/null 2>&1 || {
      # Fallback: flip ENABLED=yes in /etc/ufw/ufw.conf so boot enables it.
      [ -f /etc/ufw/ufw.conf ] && sed -i 's/^ENABLED=.*/ENABLED=yes/' /etc/ufw/ufw.conf
    }
  else
    warn "ufw not installed; skipping firewall configuration"
  fi
}

# install_plymouth_theme — a minimal splash showing "ClaudeOS" on black. We
# create a script-free text theme by deriving from the stock 'text' theme and
# only change nothing risky; if plymouth tooling is missing we skip silently.
install_plymouth_theme() {
  # The boot splash: a white screen with "ClaudeOS" in black, like ClaudeOS's own screen. Ubuntu
  # picks the splash with the default.plymouth alternative; the theme is a small Plymouth script
  # (script.so ships with plymouth, text drawing with plymouth-label).
  local themedir=/usr/share/plymouth/themes/claudeos
  if [ "$DRY_RUN" -eq 1 ]; then log "[dry-run] install ClaudeOS plymouth theme"; return 0; fi
  if ! ls /usr/lib/*/plymouth/script.so /usr/lib64/plymouth/script.so >/dev/null 2>&1; then
    log "plymouth's script plugin is absent; keeping the stock splash"
    return 0
  fi
  install -d -m 0755 "$themedir"
  printf '%s\n' '[Plymouth Theme]' 'Name=ClaudeOS' 'Description=White screen with the word ClaudeOS' \
    'ModuleName=script' '' '[script]' "ImageDir=$themedir" "ScriptFile=$themedir/claudeos.script" \
    > "$themedir/claudeos.plymouth"
  printf '%s\n' \
    'Window.SetBackgroundTopColor(1, 1, 1);' \
    'Window.SetBackgroundBottomColor(1, 1, 1);' \
    'word = Sprite(Image.Text("ClaudeOS", 0, 0, 0, 1, "Sans 22"));' \
    'word.SetX(Window.GetX() + Window.GetWidth() / 2 - word.GetImage().GetWidth() / 2);' \
    'word.SetY(Window.GetY() + Window.GetHeight() / 2 - word.GetImage().GetHeight() / 2);' \
    > "$themedir/claudeos.script"
  chmod 0644 "$themedir/claudeos.plymouth" "$themedir/claudeos.script"
  if [ "$DISTRO" = fedora ]; then
    # Fedora picks the splash with plymouth-set-default-theme; -R rebuilds the initramfs with it.
    if plymouth-set-default-theme claudeos -R >/dev/null 2>&1; then
      log "ClaudeOS boot splash set"
    else
      warn "could not set the boot splash (continuing with the stock one)"
    fi
    return 0
  fi
  if update-alternatives --install /usr/share/plymouth/themes/default.plymouth default.plymouth \
       "$themedir/claudeos.plymouth" 150 >/dev/null 2>&1 \
     && update-alternatives --set default.plymouth "$themedir/claudeos.plymouth" >/dev/null 2>&1; then
    # The splash runs from the initramfs, so rebuild it with the theme inside.
    if command -v update-initramfs >/dev/null 2>&1; then
      update-initramfs -u >/dev/null 2>&1 || warn "update-initramfs failed; the splash changes after the next kernel update"
    fi
    log "ClaudeOS boot splash set"
  else
    warn "could not set the boot splash (continuing with the stock one)"
  fi
}

# mount_mac_drive — a Mac: the drive macOS and ClaudeOS share (exFAT, "ClaudeOS" in macOS's Finder), with
# the folders ClaudeOS Setup sent over from macOS, mounted at ~/Mac. ClaudeOS's first start on the Mac
# (mac/esp/firstboot.sh) wrote its GPT partition id to /etc/claudeos/mac-files-partuuid.
mount_mac_drive() {
  local id_file=/etc/claudeos/mac-files-partuuid id uid gid line fstab=/etc/fstab tmp
  [ -r "$id_file" ] || { log "no ClaudeOS drive recorded (setup brought no folders); skipping ~/Mac"; return 0; }
  id="$(tr -d ' \n' < "$id_file")"
  [[ "$id" =~ ^[0-9a-f-]{36}$ ]] || { warn "bad drive id in $id_file; skipping ~/Mac"; return 0; }
  if [ "$DRY_RUN" -eq 1 ]; then log "[dry-run] mount PARTUUID=$id at $USER_HOME/Mac (fstab)"; return 0; fi
  uid="$(id -u "$USER_NAME")" gid="$(id -g "$USER_NAME")"
  install -d -o "$USER_NAME" -g "$USER_NAME" -m 0700 "$USER_HOME/Mac"
  line="PARTUUID=$id $USER_HOME/Mac exfat uid=$uid,gid=$gid,umask=077,nofail,x-systemd.device-timeout=10s 0 0"
  tmp="$(mktemp)"
  { grep -v -e "[[:space:]]$USER_HOME/Mac[[:space:]]" -e '^# ClaudeOS: the drive macOS and ClaudeOS share' "$fstab" 2>/dev/null || true
    printf '# ClaudeOS: the drive macOS and ClaudeOS share (ClaudeOS in macOS'"'"'s Finder)\n%s\n' "$line"
  } > "$tmp"
  if ! cmp -s "$tmp" "$fstab"; then install -m 0644 "$tmp" "$fstab"; fi
  rm -f "$tmp"
  log "the ClaudeOS drive mounts at $USER_HOME/Mac"
}

# quiet_fedora_grub — a Mac: GRUB starts the newest kernel straight away, without its menu, quietly into the
# splash. Its config is regenerated; existing boot entries get "quiet rhgb".
quiet_fedora_grub() {
  local f=/etc/default/grub
  if [ "$DRY_RUN" -eq 1 ]; then log "[dry-run] GRUB_TIMEOUT=0, hidden menu, quiet rhgb; grub2-mkconfig"; return 0; fi
  [ -f "$f" ] || : > "$f"
  set_conf "$f" GRUB_TIMEOUT 0
  set_conf "$f" GRUB_TIMEOUT_STYLE hidden
  if command -v grubby >/dev/null 2>&1; then
    grubby --update-kernel=ALL --args="quiet rhgb" >/dev/null 2>&1 || warn "grubby couldn't add quiet rhgb"
  fi
  if command -v grub2-mkconfig >/dev/null 2>&1; then
    grub2-mkconfig -o /boot/grub2/grub.cfg >/dev/null 2>&1 || warn "grub2-mkconfig failed (the menu may still show)"
  fi
}

# set_conf FILE KEY VALUE — KEY=VALUE in a shell-style config file, replacing an existing KEY= line.
set_conf() {
  local f=$1 k=$2 v=$3
  if grep -q "^$k=" "$f"; then sed -i "s|^$k=.*|$k=$v|" "$f"; else printf '%s=%s\n' "$k" "$v" >> "$f"; fi
}

# fedora_firewall — a Mac: firewalld's default zone without the SSH and Cockpit openings Fedora's server-style
# images come with (DHCPv6 stays, for the network). Written offline, so it works before firewalld runs.
fedora_firewall() {
  local zone svc
  if [ "$DRY_RUN" -eq 1 ]; then log "[dry-run] firewalld: remove ssh and cockpit from the default zone"; return 0; fi
  command -v firewall-offline-cmd >/dev/null 2>&1 || { warn "firewall-offline-cmd missing; skipping firewall"; return 0; }
  zone="$(firewall-offline-cmd --get-default-zone 2>/dev/null)"
  for svc in ssh cockpit; do
    firewall-offline-cmd --zone="${zone:-public}" --remove-service="$svc" >/dev/null 2>&1 || true
  done
}

log "ClaudeOS installer: user=$USER_NAME from_installer=$FROM_INSTALLER dry_run=$DRY_RUN skip_models=$SKIP_MODELS distro=$DISTRO"
log "repo: $APP_DIR"

# ===========================================================================
# 1) Packages.
# ===========================================================================
if [ "$DISTRO" = fedora ]; then
# Fedora Asahi Remix's Minimal image already has the Mac's own parts (Asahi's audio profiles and speaker
# safety, its firmware tools, Mesa), PipeWire, BlueZ, NetworkManager with Wi-Fi, firewalld and plymouth.
# This adds the same set as on Ubuntu below, by Fedora's names. Weak dependencies off: nothing extra.
PACKAGES=(
  # --- Display: Xorg with NO desktop environment. ---
  xorg-x11-server-Xorg       # the X server itself
  xorg-x11-drv-libinput      # keyboard, trackpad and mouse
  xorg-x11-xinit             # startx
  xset xsetroot              # disable DPMS, set black root
  openbox                    # tiny WM: borderless, maximized windows, no menu
  xdotool wmctrl             # screen/window control for the runtime
  unclutter-xfixes           # hide the idle mouse cursor (voice-first)
  xterm                      # the recovery screen, shown if ClaudeOS keeps failing to start

  # --- ClaudeOS's own screen: GTK 3 + WebKitGTK via python3-gobject. ---
  python3-gobject gtk3 webkit2gtk4.1
  gstreamer1-plugins-good gstreamer1-plugins-bad-free gstreamer1-plugin-libav gstreamer1-plugin-openh264
  poppler-utils              # pdftoppm: show PDFs as their real pages

  # --- Audio + Bluetooth (AirPods). ---
  pipewire pipewire-pulseaudio pipewire-alsa wireplumber bluez
  portaudio                  # PortAudio runtime (sounddevice)
  espeak-ng                  # fallback/phonemiser TTS

  # --- The Mac: its start-up volume ("switch to macOS") and the drive macOS shares. ---
  asahi-bless                # choose the Mac's start-up volume from Linux
  exfatprogs                 # the ClaudeOS drive is exFAT, which macOS opens too

  # --- Fonts (so the UI and emoji render). ---
  google-noto-sans-fonts google-noto-serif-fonts google-noto-sans-cjk-fonts google-noto-color-emoji-fonts
  dejavu-sans-fonts

  # --- Trust, fetching, security. ---
  ca-certificates curl sudo
  firewalld                  # host firewall (nothing incoming but what the network itself needs)
  dnf5-plugin-automatic      # automatic security updates

  # --- Boot UX. ---
  plymouth plymouth-plugin-script plymouth-plugin-label
)
log "installing ${#PACKAGES[@]} Fedora packages (minimal set)"
retry dnf install -y --setopt=install_weak_deps=False "${PACKAGES[@]}" \
  || needed "dnf install failed: ClaudeOS's packages are not all installed"
else
# The minimal set. Justification per group in comments. --no-install-recommends
# everywhere: we want nothing beyond these. Any package already installed is a
# no-op, so this stays idempotent.
PACKAGES=(
  # --- Display: Xorg with NO desktop environment. ---
  xserver-xorg-core          # the X server itself
  xserver-xorg-input-libinput # input keyboard/mouse/touch driver
  xserver-xorg-legacy        # Xorg.wrap: lets the console (autologin) user start X rootlessly
  xinit                      # provides startx/xinit
  x11-xserver-utils          # xset/xsetroot: disable DPMS, set black root
  openbox                    # tiny WM: borderless, maximized windows, no menu
  xdotool                    # screen/window control for the runtime
  wmctrl                     # window management for the runtime
  unclutter-xfixes           # hide the idle mouse cursor (voice-first)
  xterm                      # the recovery screen, shown if ClaudeOS keeps failing to start

  # --- ClaudeOS's own screen: GTK 3 + WebKitGTK via python3-gi. ---
  python3-venv               # create /opt/claudeos/venv
  python3-gi                 # GObject introspection for GTK/WebKit from Python
  gir1.2-webkit2-4.1         # WebKitGTK 4.1 typelib (the web view)
  gir1.2-gtk-3.0             # GTK 3 typelib (the ClaudeOS window)
  # Web media codecs so pages/video render inside the web view:
  gstreamer1.0-plugins-good
  gstreamer1.0-plugins-bad
  gstreamer1.0-libav
  gstreamer1.0-gl            # GStreamer GL elements WebKit uses for video
  libgl1-mesa-dri            # GPU drivers for OpenGL (AMD/Intel) so pages and video are smooth
  libegl1                    # EGL, used by WebKitGTK's GPU rendering
  poppler-utils              # pdftoppm: show PDFs as their real pages

  # --- Audio + Bluetooth (AirPods). ---
  alsa-ucm-conf              # mixer profiles many onboard sound chips need
  pipewire                   # audio server
  pipewire-pulse             # PulseAudio-compatible endpoint (apps expect it)
  pipewire-alsa              # ALSA apps route through PipeWire
  wireplumber                # PipeWire session/policy manager
  libspa-0.2-bluetooth       # Bluetooth audio (A2DP/HFP, mSBC) for PipeWire
  bluez                      # the Bluetooth stack (bluetoothd)
  dbus-user-session          # per-user D-Bus (PipeWire/WirePlumber user services)

  # --- Networking (no admin; NM owns everything). ---
  network-manager            # Wi-Fi/Ethernet management
  wpasupplicant              # WPA for Wi-Fi (NM backend)

  # --- Windows files + dual-boot plumbing. ---
  ntfs-3g                    # read/write NTFS (Windows partition)
  cryptsetup-bin             # open Windows' BitLocker drive with ClaudeOS's own key (cryptsetup BITLK)
  efibootmgr                 # UEFI BootNext ("switch to Windows")

  # --- Voice I/O helpers for the runtime. ---
  libportaudio2              # PortAudio runtime (sounddevice)
  espeak-ng                  # fallback/phonemiser TTS

  # --- Fonts (so the UI and emoji render). ---
  fonts-noto-core
  fonts-noto-cjk             # Chinese/Japanese/Korean text on web pages and in documents
  fonts-noto-color-emoji
  fonts-dejavu-core

  # --- Trust, fetching, security. ---
  pciutils                   # lspci: detect the graphics card (driver choice)
  ca-certificates            # TLS trust for HTTPS to Claude
  curl                       # fetching (model downloads etc.)
  sudo                       # the root helper is reached via `sudo -n`; visudo validates the drop-in
  ufw                        # host firewall (default-deny inbound)
  unattended-upgrades        # automatic security updates

  # --- Boot UX (optional polish; cheap & useful). ---
  plymouth                   # boot splash framework
  plymouth-label             # text rendering for the splash
)

log "installing ${#PACKAGES[@]} apt packages (minimal set)"
export DEBIAN_FRONTEND=noninteractive
# `apt-get update` then install, each retried for transient network errors.
retry apt-get update || warn "apt-get update failed; continuing with cached lists"
retry apt-get install -y --no-install-recommends "${PACKAGES[@]}" \
  || needed "apt-get install failed: ClaudeOS's packages are not all installed"

# GPU drivers: only pull proprietary NVIDIA drivers when an NVIDIA GPU is
# actually present. AMD (this machine may be AMD) uses the in-kernel amdgpu
# driver already shipped — nothing to install. Guard so a failure here never
# aborts the whole install.
if command -v lspci >/dev/null 2>&1 && lspci -nn 2>/dev/null | grep -iqE 'VGA|3D|Display' ; then
  if lspci -nn 2>/dev/null | grep -iq 'NVIDIA'; then
    log "NVIDIA GPU detected; installing recommended drivers via ubuntu-drivers"
    if ! command -v ubuntu-drivers >/dev/null 2>&1; then
      retry apt-get install -y --no-install-recommends ubuntu-drivers-common || warn "could not install ubuntu-drivers-common"
    fi
    # ubuntu-drivers understands --gpgpu/autoinstall; in a chroot it stages the
    # driver packages for first boot. Never fatal.
    run_sh 'ubuntu-drivers install' || warn "ubuntu-drivers install failed; NVIDIA driver not installed (continuing)"
  else
    log "non-NVIDIA GPU detected; relying on in-kernel driver (amdgpu/i915). No extra drivers."
  fi
else
  log "no discrete GPU detected via lspci (or lspci missing); skipping driver install"
fi

fi

# ===========================================================================
# 2) Python venv for the runtime.
# ===========================================================================
# --system-site-packages so the venv can import the distro's python3-gi / GI
# typelibs (WebKit, GTK) which are not on PyPI.
if [ ! -x "$VENV_DIR/bin/python" ]; then
  log "creating venv at $VENV_DIR (--system-site-packages)"
  run python3 -m venv --system-site-packages "$VENV_DIR" || needed "could not create the Python environment at $VENV_DIR"
else
  log "venv already present at $VENV_DIR"
fi

if [ -f "$REQ_FILE" ] || [ "$DRY_RUN" -eq 1 ]; then
  log "installing Python requirements from $REQ_FILE"
  retry "$VENV_DIR/bin/pip" install --upgrade pip || warn "pip self-upgrade failed (continuing)"
  retry "$VENV_DIR/bin/pip" install -r "$REQ_FILE" || needed "pip install failed: the ClaudeOS runtime is incomplete"
else
  needed "requirements file $REQ_FILE not found"
fi

# ClaudeOS itself isn't installed with pip: a .pth file puts $OPT_APP on the venv's import path, so
# `python -m claudeos` works from any folder (the session starts it from the home folder).
if [ "$DRY_RUN" -eq 0 ]; then
  SITE_DIR="$("$VENV_DIR/bin/python" -c 'import sysconfig; print(sysconfig.get_paths()["purelib"])')" \
    || needed "could not find the package folder of $VENV_DIR"
  printf '%s\n' "$OPT_APP" > "$SITE_DIR/claudeos-app.pth"
  (cd / && "$VENV_DIR/bin/python" -c 'import claudeos.config') \
    || needed "the Python environment can't load ClaudeOS from $OPT_APP"
  log "ClaudeOS loads from $OPT_APP"
else
  log "[dry-run] point $VENV_DIR at $OPT_APP (claudeos-app.pth)"
fi

# ===========================================================================
# 3) User groups + home layout.
# ===========================================================================
# The autologin session user needs hardware access: audio (PipeWire/ALSA),
# video (DRM/GPU for X), input (libinput devices under rootless X), bluetooth
# (bluetoothd control), netdev (NM) and render (DRI). Groups that do not exist
# on this release are skipped.
for grp in audio video input bluetooth netdev render plugdev; do
  if getent group "$grp" >/dev/null 2>&1; then
    if id -nG "$USER_NAME" 2>/dev/null | tr ' ' '\n' | grep -qx "$grp"; then
      : # already a member
    else
      log "adding $USER_NAME to group $grp"
      run usermod -aG "$grp" "$USER_NAME"
    fi
  fi
done

# Home data tree owned by the user.
if [ "$DRY_RUN" -eq 0 ]; then
  install -d -o "$USER_NAME" -g "$USER_NAME" -m 0755 "$CLAUDEOS_HOME" \
    "$CLAUDEOS_HOME/logs" "$CLAUDEOS_HOME/models" "$CLAUDEOS_HOME/secrets"
  chmod 0700 "$CLAUDEOS_HOME/secrets"
else
  log "[dry-run] create $CLAUDEOS_HOME {logs,models,secrets}"
fi

# ===========================================================================
# 4) Session: autologin + startx + session script.
# ===========================================================================
log "configuring tty1 autologin and X session"
install_templated "$SYSTEM_DIR/etc/systemd/system/getty@tty1.service.d/10-claudeos-autologin.conf" \
  "/etc/systemd/system/getty@tty1.service.d/10-claudeos-autologin.conf" 0644

# bash_profile (startx on tty1) and xinitrc (exec session) live in the user's
# home, owned by the user.
install_file "$LINUX_DIR/session/bash_profile" "$USER_HOME/.bash_profile" 0644 "$USER_NAME:$USER_NAME"
install_file "$LINUX_DIR/session/xinitrc"      "$USER_HOME/.xinitrc"      0644 "$USER_NAME:$USER_NAME"
# An empty ~/.hushlogin: no "Welcome to Ubuntu" text on the screen between the splash and ClaudeOS.
install_file "$LINUX_DIR/session/hushlogin"    "$USER_HOME/.hushlogin"    0644 "$USER_NAME:$USER_NAME"

# Make the session scripts executable in the repo copy (they run from there).
if [ "$DRY_RUN" -eq 0 ]; then
  chmod 0755 "$LINUX_DIR/session/claudeos-session" "$LINUX_DIR/session/claudeos-restart" 2>/dev/null || true
fi

# Rootless X for the console user (Ubuntu's Xorg.wrap; Fedora's Xorg gets the devices from logind).
if [ "$DISTRO" = ubuntu ]; then
  install_file "$SYSTEM_DIR/etc/X11/Xwrapper.config" "/etc/X11/Xwrapper.config" 0644
fi

# getty@tty1 is already enabled by default; our drop-in just adds autologin.
# Ensure it is enabled (symlink) so first boot has it.
sctl_enable "getty@tty1.service"

# ===========================================================================
# 5) Audio / Bluetooth config.
# ===========================================================================
log "installing WirePlumber + BlueZ configuration"
if [ "$DISTRO" = fedora ]; then
  # Fedora's WirePlumber is 0.5, configured in SPA-JSON; Ubuntu's 0.4 in Lua. The same settings.
  install_file "$SYSTEM_DIR/etc/wireplumber/wireplumber.conf.d/51-claudeos-bluetooth.conf" \
    "/etc/wireplumber/wireplumber.conf.d/51-claudeos-bluetooth.conf" 0644
else
  install_file "$SYSTEM_DIR/etc/wireplumber/wireplumber.conf.d/51-claudeos-bluetooth.lua" \
    "/etc/wireplumber/wireplumber.conf.d/51-claudeos-bluetooth.lua" 0644
  install_file "$SYSTEM_DIR/etc/wireplumber/wireplumber.conf.d/51-claudeos-policy.lua" \
    "/etc/wireplumber/wireplumber.conf.d/51-claudeos-policy.lua" 0644
fi

# BlueZ 5.72 on noble does not read main.conf.d, so apply our settings directly
# into /etc/bluetooth/main.conf, idempotently, via a marked block. We keep the
# reference drop-in on disk for documentation / future BlueZ.
install_file "$SYSTEM_DIR/etc/bluetooth/main.conf.d/10-claudeos.conf" \
  "/etc/bluetooth/main.conf.d/10-claudeos.conf" 0644
install_file "$SYSTEM_DIR/etc/bluetooth/main.conf.d/90-claudeos-bredr-fallback.conf.disabled" \
  "/etc/bluetooth/main.conf.d/90-claudeos-bredr-fallback.conf.disabled" 0644
apply_bluez_main_conf

# Enable the per-user audio stack so it starts in the session. In a chroot we
# can only enable; `systemctl --global enable` works without a running systemd
# and turns the user services on for every user login.
run systemctl --global enable wireplumber.service pipewire.service pipewire-pulse.socket 2>/dev/null \
  || warn "could not --global enable pipewire/wireplumber (will still autostart via dbus)"
sctl_enable "bluetooth.service"

# PipeWire/WirePlumber as user services need lingering off (they start with the
# graphical session), but enabling dbus-user-session is enough; nothing to do.

# ===========================================================================
# 6) Networking: NetworkManager manages everything; netplan renderer = NM.
# ===========================================================================
log "configuring NetworkManager as the sole network manager"
install_file "$SYSTEM_DIR/etc/NetworkManager/conf.d/10-claudeos-manage-all.conf" \
  "/etc/NetworkManager/conf.d/10-claudeos-manage-all.conf" 0644
if [ "$DISTRO" = ubuntu ]; then
# netplan: set the global renderer to NetworkManager and neutralise the
# installer's networkd config so NM owns the links.
install_file "$SYSTEM_DIR/etc/netplan/99-claudeos-nm.yaml" \
  "/etc/netplan/99-claudeos-nm.yaml" 0600
neutralise_installer_netplan
fi
sctl_enable "NetworkManager.service"
# systemd-networkd has nothing left to manage, and its wait-online service would hold up every
# start by two minutes, waiting for a network it doesn't manage.
for unit in systemd-networkd-wait-online.service systemd-networkd.service systemd-networkd.socket; do
  run systemctl disable "$unit" >/dev/null 2>&1 || true
done
# In a chroot `disable` can silently do nothing (see sctl_enable); remove the links ourselves.
[ "$DRY_RUN" -eq 1 ] || find /etc/systemd/system -path '*.wants/*' \( -name systemd-networkd-wait-online.service \
  -o -name systemd-networkd.service -o -name systemd-networkd.socket \) -delete
# Generate backend config from netplan (safe in chroot: just writes files).
[ "$DISTRO" = ubuntu ] && { run_sh 'netplan generate' || warn "netplan generate failed (continuing)"; }

# ===========================================================================
# 7) The other system's files: Windows' drive (a PC), or the drive macOS shares (a Mac).
# ===========================================================================
if [ "$DISTRO" = ubuntu ]; then
  log "installing Windows auto-mount service"
  install_file "$SYSTEM_DIR/etc/systemd/system/claudeos-windows-mount.service" \
    "/etc/systemd/system/claudeos-windows-mount.service" 0644
  run install -d -m 0755 /mnt/windows
  sctl_enable "claudeos-windows-mount.service"
else
  mount_mac_drive
fi

# ===========================================================================
# 8) Root helper + sudoers + BT import script.
# ===========================================================================
log "installing root helper and sudoers drop-in"
install_file "$LINUX_DIR/root-helper/claudeos-root" "$LIBEXEC_DIR/claudeos-root" 0755 "root:root"
install_file "$LINUX_DIR/root-helper/bt-import-windows-keys.py" \
  "$LIBEXEC_DIR/bt-import-windows-keys.py" 0755 "root:root"

# sudoers: render the user, then validate with visudo BEFORE moving into place.
# A bad sudoers file can lock out privilege escalation, so we never install one
# that visudo rejects.
if [ "$DRY_RUN" -eq 1 ]; then
  log "[dry-run] render + visudo-check /etc/sudoers.d/claudeos"
else
  tmp_sudoers="$(mktemp)"
  sed "s/@CLAUDEOS_USER@/$USER_NAME/g" "$SYSTEM_DIR/etc/sudoers.d/claudeos" > "$tmp_sudoers"
  if visudo -cf "$tmp_sudoers" >/dev/null 2>&1; then
    install -m 0440 -o root -g root "$tmp_sudoers" /etc/sudoers.d/claudeos
    log "installed validated /etc/sudoers.d/claudeos"
  else
    warn "generated sudoers failed visudo -cf; NOT installing it"
    visudo -cf "$tmp_sudoers" >&2 || true
  fi
  rm -f "$tmp_sudoers"
fi

# ===========================================================================
# 9) GRUB boot menu.
# ===========================================================================
if [ "$DISTRO" = fedora ]; then
  # A Mac chooses between macOS and ClaudeOS itself (its startup options, asahi-bless), so GRUB only
  # starts Fedora's newest kernel, without showing its menu, and quietly into the splash.
  log "hiding the GRUB menu"
  quiet_fedora_grub
else
log "installing GRUB entries (ClaudeOS default, Windows)"
install_file "$LINUX_DIR/grub/07_claudeos" "/etc/grub.d/07_claudeos" 0755 "root:root"
install_file "$SYSTEM_DIR/etc/default/grub.d/99-claudeos.cfg" \
  "/etc/default/grub.d/99-claudeos.cfg" 0644
# Set the saved default to our entry id, then regenerate grub.cfg. Guard both
# for chroot/dry-run: grub-set-default/update-grub touch /boot and want a
# grub-probe-able environment; in a chroot bind-mounts make this work, but we
# still tolerate failure so the install completes (first boot / a manual
# update-grub can finish it).
if command -v grub-set-default >/dev/null 2>&1; then
  run grub-set-default "ClaudeOS" || warn "grub-set-default failed (will default on first update-grub)"
fi
if command -v update-grub >/dev/null 2>&1; then
  run update-grub || warn "update-grub failed in this environment (run it on the target)"
elif command -v grub-mkconfig >/dev/null 2>&1; then
  run_sh 'grub-mkconfig -o /boot/grub/grub.cfg' || warn "grub-mkconfig failed (run it on the target)"
fi
fi

# ===========================================================================
# 10) Firewall + security.
# ===========================================================================
if [ "$DISTRO" = fedora ]; then
  log "configuring firewalld (nothing incoming but what the network needs) and automatic security updates"
  fedora_firewall
  sctl_enable "firewalld.service"
  install_file "$SYSTEM_DIR/etc/dnf/automatic.conf" "/etc/dnf/automatic.conf" 0644
  sctl_enable "dnf5-automatic.timer"
  # No SSH server (contract: no SSH). Fedora's Minimal image comes with one turned on.
  if [ -f /usr/lib/systemd/system/sshd.service ]; then
    log "turning off the SSH server (contract: no SSH server)"
    run systemctl disable --now sshd.service sshd.socket 2>/dev/null || run systemctl disable sshd.service 2>/dev/null || true
    [ "$DRY_RUN" -eq 1 ] || find /etc/systemd/system -path '*.wants/*' \( -name sshd.service -o -name sshd.socket \) -delete
  fi
else
log "configuring ufw (default deny incoming, allow outgoing) and unattended-upgrades"
# Set policy via config edits (works in chroot; `ufw enable` needs iptables and
# is applied at boot by the ufw unit).
set_ufw_defaults
sctl_enable "ufw.service"

install_file "$SYSTEM_DIR/etc/apt/apt.conf.d/52claudeos-unattended-upgrades" \
  "/etc/apt/apt.conf.d/52claudeos-unattended-upgrades" 0644
sctl_enable "unattended-upgrades.service"

# Explicitly make sure there is NO SSH server (contract: no SSH). We do not
# install openssh-server; if something pulled it in, disable it.
if dpkg -s openssh-server >/dev/null 2>&1; then
  warn "openssh-server is installed; disabling it (contract: no SSH server)"
  run systemctl disable --now ssh.service 2>/dev/null || run systemctl disable ssh.service 2>/dev/null || true
fi
fi

# ===========================================================================
# 11) Plymouth splash (optional polish).
# ===========================================================================
install_plymouth_theme

# ===========================================================================
# 12) Voice models.
# ===========================================================================
if [ "$SKIP_MODELS" -eq 1 ]; then
  log "--skip-models: not downloading voice models"
elif [ "$FROM_INSTALLER" -eq 1 ]; then
  log "running under installer: deferring model download to first boot (run as the user)"
else
  if [ -x "$VENV_DIR/bin/python" ] && [ -f "$REQ_FILE" ]; then
    log "downloading voice models as $USER_NAME"
    run_sh "runuser -u '$USER_NAME' -- '$VENV_DIR/bin/python' -P -m claudeos models --download" \
      || warn "model download failed (can be retried later)"
  fi
fi

# ===========================================================================
# Done.
# ===========================================================================
# Fedora checks SELinux labels: give everything written above the labels its policy says.
if [ "$DISTRO" = fedora ] && [ "$DRY_RUN" -eq 0 ] && command -v restorecon >/dev/null 2>&1; then
  restorecon -RF /etc /opt/claudeos /usr/local "$USER_HOME" /var/lib/claudeos >/dev/null 2>&1 \
    || warn "restorecon reported problems (continuing)"
fi
log "ClaudeOS base install complete."
if [ "$FROM_INSTALLER" -eq 1 ]; then
  log "services enabled; they start on first boot into the target."
fi
# Mark the base as installed. claudeos-finish-install.service (the safety net the installer's
# finish.sh sets up) only runs until this exists; turn it off as well.
if [ "$DRY_RUN" -eq 0 ] && [ ! -f "$STAMP" ]; then
  install -d -m 0755 "$(dirname "$STAMP")"
  date -u +%Y-%m-%dT%H:%M:%SZ > "$STAMP"
fi
if [ -e /etc/systemd/system/claudeos-finish-install.service ]; then
  run systemctl disable claudeos-finish-install.service >/dev/null 2>&1 || true
  # In a chroot `disable` can silently do nothing (see sctl_enable); remove the link ourselves.
  [ "$DRY_RUN" -eq 1 ] || find /etc/systemd/system -path '*.wants/*' -name claudeos-finish-install.service \
    -type l -delete 2>/dev/null || true
fi
exit 0
