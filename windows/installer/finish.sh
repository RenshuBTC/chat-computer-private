#!/bin/sh
# ClaudeOS finishing step. Runs in the Ubuntu installer after Ubuntu has been installed to
# /target (autoinstall late-commands): copies ClaudeOS and the person's sign-in and settings from
# the installer partition, then turns the new system into ClaudeOS (linux/install-claudeos.sh).
# Usage: finish.sh USER INSTALL_ID
set -eu
USER_NAME="$1"
INSTALL_ID="$2"
SRC="${CLAUDEOS_FINISH_SRC:-/cdrom/claudeos}"
T="${CLAUDEOS_FINISH_TARGET:-/target}"

log() { echo "claudeos-finish: $*"; }

case "$USER_NAME" in
  ''|*[!a-z0-9_-]*) echo "claudeos-finish: bad user name" >&2; exit 2 ;;
esac
case "$INSTALL_ID" in
  ''|*[!0-9a-f]*) echo "claudeos-finish: bad install id" >&2; exit 2 ;;
esac
[ -d "$SRC/app" ] || { echo "claudeos-finish: $SRC/app is missing" >&2; exit 2; }

# 1. The program, owned by root and read-only to the user. The installer partition is FAT32, which has no
#    permissions, so set them: directories and files readable, scripts executable.
log "copying ClaudeOS"
mkdir -p "$T/opt/claudeos"
rm -rf "$T/opt/claudeos/app.new"
cp -r "$SRC/app" "$T/opt/claudeos/app.new"
chmod -R u=rwX,go=rX "$T/opt/claudeos/app.new"
find "$T/opt/claudeos/app.new" -type f -exec chmod a-x {} +
# Anything starting with "#!" is a program.
find "$T/opt/claudeos/app.new" -type f -exec sh -c \
  'for f in "$@"; do if head -c 2 "$f" | grep -q "^#!"; then chmod 0755 "$f"; fi; done' sh {} +
rm -rf "$T/opt/claudeos/app"
mv "$T/opt/claudeos/app.new" "$T/opt/claudeos/app"
chown -R 0:0 "$T/opt/claudeos/app"

# 2. The person's sign-in and settings, owned by them.
H="$T/home/$USER_NAME/ClaudeOS"
mkdir -p "$H/secrets"
chmod 0700 "$H/secrets"
if [ -f "$SRC/secrets/claude_token" ]; then
  cp "$SRC/secrets/claude_token" "$H/secrets/claude_token"
  chmod 0600 "$H/secrets/claude_token"
fi
# The full sign-in for the Claude app on the phone goes where Claude Code reads (and refreshes) it.
if [ -f "$SRC/secrets/claude_login.json" ]; then
  mkdir -p "$H/.engine"
  chmod 0700 "$H/.engine"
  cp "$SRC/secrets/claude_login.json" "$H/.engine/.credentials.json"
  chmod 0600 "$H/.engine/.credentials.json"
fi
if [ -f "$SRC/settings.json" ]; then
  cp "$SRC/settings.json" "$H/settings.json"
  chmod 0644 "$H/settings.json"     # FAT32, where every file looks executable
fi
curtin in-target --target="$T" -- chown -R "$USER_NAME:$USER_NAME" "/home/$USER_NAME/ClaudeOS"
# ClaudeOS's own BitLocker key for Windows' drive (when it's encrypted): root only. ClaudeOS opens Windows'
# drive with it (claudeos-root mount-windows).
if [ -f "$SRC/secrets/bitlocker-key" ]; then
  mkdir -p "$T/etc/claudeos"
  ( umask 077; cp "$SRC/secrets/bitlocker-key" "$T/etc/claudeos/bitlocker-recovery-key" )
  chown 0:0 "$T/etc/claudeos/bitlocker-recovery-key"
  chmod 0600 "$T/etc/claudeos/bitlocker-recovery-key"
fi

# 3. Mark this installer as used on this PC, before the long step below. If the PC starts the installer
#    again, its boot menu finds this file and starts ClaudeOS instead of installing it again (guard.sh
#    checks it too).
#    It also names ClaudeOS's partition (its GPT partition GUID), so "Remove ClaudeOS" in ClaudeOS Setup on
#    Windows deletes exactly that one.
if [ -d "$T/boot/efi/EFI" ]; then
  mkdir -p "$T/boot/efi/EFI/claudeos"
  root_dev=$(findmnt -n -o SOURCE "$T" 2>/dev/null || true)
  root_part=""
  [ -n "$root_dev" ] && root_part=$(blkid -s PARTUUID -o value "$root_dev" 2>/dev/null || true)
  {
    printf 'id=%s\n' "$INSTALL_ID"
    if [ -n "$root_part" ]; then printf 'root=%s\n' "$root_part"; fi
  } > "$T/boot/efi/EFI/claudeos/installed-$INSTALL_ID"
fi

# 4. A safety net for step 5: if it can't finish now (no internet, a power cut), ClaudeOS finishes
#    installing itself on the first start, before anything else (linux/firstboot). It does nothing
#    once step 5 has worked.
#    And the clean-up: on its first start ClaudeOS deletes this installer's partition (the one with
#    claudeos/installer-<id>) and grows into its space (linux/firstboot/claudeos-reclaim-installer).
mkdir -p "$T/etc/claudeos" "$T/etc/systemd/system/multi-user.target.wants"
printf '%s\n' "$USER_NAME" > "$T/etc/claudeos/install-user"
printf '%s\n' "$INSTALL_ID" > "$T/etc/claudeos/install-id"
for unit in claudeos-finish-install.service claudeos-reclaim-installer.service; do
  cp "$T/opt/claudeos/app/linux/firstboot/$unit" "$T/etc/systemd/system/"
  # Enabling a unit is this one link; made directly, because systemctl can silently do nothing here.
  ln -sf "/etc/systemd/system/$unit" "$T/etc/systemd/system/multi-user.target.wants/$unit"
done

# 5. Turn Ubuntu into ClaudeOS. It downloads packages, so try three times; if it still fails,
#    let the Ubuntu install end normally and leave the rest to the safety net.
WAIT="${CLAUDEOS_FINISH_RETRY_WAIT:-30}"
run_installer() {
  curtin in-target --target="$T" -- bash /opt/claudeos/app/linux/install-claudeos.sh --user "$USER_NAME" --from-installer
}
try=1
until run_installer; do
  if [ "$try" -ge 3 ]; then
    log "couldn't finish now; ClaudeOS will finish installing when the computer starts"
    exit 0
  fi
  log "try $try failed; trying again in $((WAIT * try)) seconds"
  sleep $((WAIT * try))
  try=$((try + 1))
done
log "done"
