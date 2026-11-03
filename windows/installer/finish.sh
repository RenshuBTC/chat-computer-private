#!/bin/sh
# Chat Computer finishing step. Runs in the Ubuntu installer after Ubuntu has been installed to
# /target (autoinstall late-commands): copies Chat Computer and the person's sign-in and settings from
# the installer partition, then turns the new system into Chat Computer (linux/install-chatcomputer.sh).
# Usage: finish.sh USER INSTALL_ID
set -eu
USER_NAME="$1"
INSTALL_ID="$2"
SRC="${CHATCOMPUTER_FINISH_SRC:-/cdrom/chatcomputer}"
T="${CHATCOMPUTER_FINISH_TARGET:-/target}"

log() { echo "chatcomputer-finish: $*"; }

case "$USER_NAME" in
  ''|*[!a-z0-9_-]*) echo "chatcomputer-finish: bad user name" >&2; exit 2 ;;
esac
case "$INSTALL_ID" in
  ''|*[!0-9a-f]*) echo "chatcomputer-finish: bad install id" >&2; exit 2 ;;
esac
[ -d "$SRC/app" ] || { echo "chatcomputer-finish: $SRC/app is missing" >&2; exit 2; }

# 1. The program, owned by root and read-only to the user. The installer partition is FAT32, which has no
#    permissions, so set them: directories and files readable, scripts executable.
log "copying Chat Computer"
mkdir -p "$T/opt/chatcomputer"
rm -rf "$T/opt/chatcomputer/app.new"
cp -r "$SRC/app" "$T/opt/chatcomputer/app.new"
chmod -R u=rwX,go=rX "$T/opt/chatcomputer/app.new"
find "$T/opt/chatcomputer/app.new" -type f -exec chmod a-x {} +
# Anything starting with "#!" is a program.
find "$T/opt/chatcomputer/app.new" -type f -exec sh -c \
  'for f in "$@"; do if head -c 2 "$f" | grep -q "^#!"; then chmod 0755 "$f"; fi; done' sh {} +
rm -rf "$T/opt/chatcomputer/app"
mv "$T/opt/chatcomputer/app.new" "$T/opt/chatcomputer/app"
chown -R 0:0 "$T/opt/chatcomputer/app"

# 2. The person's settings, and Chat Computer's private folder, owned by them.
H="$T/home/$USER_NAME/ChatComputer"
mkdir -p "$H/secrets"
chmod 0700 "$H/secrets"
if [ -f "$SRC/settings.json" ]; then
  cp "$SRC/settings.json" "$H/settings.json"
  chmod 0644 "$H/settings.json"     # FAT32, where every file looks executable
fi
curtin in-target --target="$T" -- chown -R "$USER_NAME:$USER_NAME" "/home/$USER_NAME/ChatComputer"
# Chat Computer's own BitLocker key for Windows' drive (when it's encrypted): root only. Chat Computer opens Windows'
# drive with it (chatcomputer-root mount-windows).
if [ -f "$SRC/secrets/bitlocker-key" ]; then
  mkdir -p "$T/etc/chatcomputer"
  ( umask 077; cp "$SRC/secrets/bitlocker-key" "$T/etc/chatcomputer/bitlocker-recovery-key" )
  chown 0:0 "$T/etc/chatcomputer/bitlocker-recovery-key"
  chmod 0600 "$T/etc/chatcomputer/bitlocker-recovery-key"
fi

# 3. Mark this installer as used on this PC, before the long step below. If the PC starts the installer
#    again, its boot menu finds this file and starts Chat Computer instead of installing it again (guard.sh
#    checks it too).
#    It also names Chat Computer's partition (its GPT partition GUID), so "Remove Chat Computer" in Chat Computer Setup on
#    Windows deletes exactly that one.
if [ -d "$T/boot/efi/EFI" ]; then
  mkdir -p "$T/boot/efi/EFI/chatcomputer"
  root_dev=$(findmnt -n -o SOURCE "$T" 2>/dev/null || true)
  root_part=""
  [ -n "$root_dev" ] && root_part=$(blkid -s PARTUUID -o value "$root_dev" 2>/dev/null || true)
  {
    printf 'id=%s\n' "$INSTALL_ID"
    if [ -n "$root_part" ]; then printf 'root=%s\n' "$root_part"; fi
  } > "$T/boot/efi/EFI/chatcomputer/installed-$INSTALL_ID"
fi

# 4. A safety net for step 5: if it can't finish now (no internet, a power cut), Chat Computer finishes
#    installing itself on the first start, before anything else (linux/firstboot). It does nothing
#    once step 5 has worked.
#    And the clean-up: on its first start Chat Computer deletes this installer's partition (the one with
#    chatcomputer/installer-<id>) and grows into its space (linux/firstboot/chatcomputer-reclaim-installer).
mkdir -p "$T/etc/chatcomputer" "$T/etc/systemd/system/multi-user.target.wants"
printf '%s\n' "$USER_NAME" > "$T/etc/chatcomputer/install-user"
printf '%s\n' "$INSTALL_ID" > "$T/etc/chatcomputer/install-id"
# Where Chat Computer Setup ran from on Windows' drive: a newer Chat Computer put there updates this one (chatcomputer-root
# update-app). Root's, so nothing Chat Computer runs can point updates anywhere else.
if [ -f "$SRC/update-source" ]; then
  cp "$SRC/update-source" "$T/etc/chatcomputer/update-source"
  chown 0:0 "$T/etc/chatcomputer/update-source"
  chmod 0644 "$T/etc/chatcomputer/update-source"
fi
for unit in chatcomputer-finish-install.service chatcomputer-reclaim-installer.service; do
  cp "$T/opt/chatcomputer/app/linux/firstboot/$unit" "$T/etc/systemd/system/"
  # Enabling a unit is this one link; made directly, because systemctl can silently do nothing here.
  ln -sf "/etc/systemd/system/$unit" "$T/etc/systemd/system/multi-user.target.wants/$unit"
done

# 5. Turn Ubuntu into Chat Computer. It downloads packages, so try three times; if it still fails,
#    let the Ubuntu install end normally and leave the rest to the safety net.
WAIT="${CHATCOMPUTER_FINISH_RETRY_WAIT:-30}"
run_installer() {
  curtin in-target --target="$T" -- bash /opt/chatcomputer/app/linux/install-chatcomputer.sh --user "$USER_NAME" --from-installer
}
try=1
until run_installer; do
  if [ "$try" -ge 3 ]; then
    log "couldn't finish now; Chat Computer will finish installing when the computer starts"
    exit 0
  fi
  log "try $try failed; trying again in $((WAIT * try)) seconds"
  sleep $((WAIT * try))
  try=$((try + 1))
done
log "done"
