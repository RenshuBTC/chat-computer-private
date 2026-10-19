#!/bin/sh
# Chat Computer install guard. Runs in the Ubuntu installer before anything is written to disk
# (autoinstall early-commands). If this installer has already installed Chat Computer on this PC, it
# must not install a second copy.
#
# finish.sh leaves /EFI/chatcomputer/installed-<id> on the PC's EFI partition. If the PC starts the installer
# again (before Chat Computer has cleaned it up), the installer's boot menu normally finds that file and starts
# the installed Chat Computer, so the installer never runs. This is the second check, for when it does run
# anyway: it makes the firmware start the installed system once (BootNext) and restarts; if it can't, it
# stops before anything changes.
# Usage: guard.sh INSTALL_ID
set -u
ID="${1:-}"
case "$ID" in
  ''|*[!0-9a-f]*) exit 0 ;;
esac
EFIVARS="${CHATCOMPUTER_EFIVARS:-/sys/firmware/efi/efivars}"
GLOBAL=8be4df61-93ca-11d2-aa0d-00e098032b8c   # UEFI's own variables (Boot####, BootNext)

say() {
  echo "$1"
  echo "$1" > /dev/console 2>/dev/null || true
}

installed_here() {
  # For tests, CHATCOMPUTER_GUARD_ESPS names folders to look in instead of the PC's EFI partitions.
  if [ -n "${CHATCOMPUTER_GUARD_ESPS:-}" ]; then
    for d in $CHATCOMPUTER_GUARD_ESPS; do
      [ -f "$d/EFI/chatcomputer/installed-$ID" ] && return 0
    done
    return 1
  fi
  for dev in $(lsblk -rpno NAME,FSTYPE 2>/dev/null | awk '$2 == "vfat" { print $1 }'); do
    mnt=$(mktemp -d) || continue
    hit=1
    if mount -o ro "$dev" "$mnt" 2>/dev/null; then
      [ -f "$mnt/EFI/chatcomputer/installed-$ID" ] && hit=0
      umount "$mnt" 2>/dev/null
    fi
    rmdir "$mnt" 2>/dev/null
    [ "$hit" -eq 0 ] && return 0
  done
  return 1
}

ubuntu_entry() {
  # The number of the firmware's boot entry named "ubuntu" (made when Ubuntu installed its boot loader).
  for f in "$EFIVARS"/Boot[0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f]-"$GLOBAL"; do
    [ -f "$f" ] || continue
    # 4 bytes of variable attributes, 4 of entry attributes, 2 of path length, then the name in UTF-16.
    name=$(dd if="$f" bs=1 skip=10 count=14 2>/dev/null | tr -d '\000')
    if [ "$name" = "ubuntu" ]; then
      n=${f##*/Boot}
      echo "${n%%-*}"
      return 0
    fi
  done
  return 1
}

set_boot_next() {
  # Start entry $1 once on the next restart, whatever the PC would start otherwise. The installer has no
  # efibootmgr, so the variable is written directly, in one write: attributes, then the entry number.
  hi=$(printf '%03o' "0x$(printf '%s' "$1" | cut -c1-2)") || return 1
  lo=$(printf '%03o' "0x$(printf '%s' "$1" | cut -c3-4)") || return 1
  var="$EFIVARS/BootNext-$GLOBAL"
  tmp=$(mktemp) || return 1
  # shellcheck disable=SC2059  # the octal escapes are built on purpose
  printf "\\007\\000\\000\\000\\$lo\\$hi" > "$tmp"
  if [ -e "$var" ]; then
    chattr -i "$var" 2>/dev/null
    rm -f "$var"
  fi
  dd if="$tmp" of="$var" bs=6 count=1 2>/dev/null
  rc=$?
  rm -f "$tmp"
  return "$rc"
}

installed_here || exit 0

entry=$(ubuntu_entry) && set_boot_next "$entry" && {
  say "Chat Computer is already installed on this PC; restarting into Chat Computer."
  if [ -n "${CHATCOMPUTER_GUARD_REBOOT:-}" ]; then  # for tests
    $CHATCOMPUTER_GUARD_REBOOT
    exit 0
  fi
  sleep 8
  systemctl reboot || reboot -f
  sleep 60
}
# Could not arrange that: stop the installer here (nothing has been changed).
say "Chat Computer is already installed on this PC, so it won't install again. Restart the PC and choose Chat Computer in its boot menu." >&2
exit 1
