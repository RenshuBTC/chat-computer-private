#!/bin/bash
# Chat Computer: give Chat Computer its installer's space. Runs once, on the first start
# (chatcomputer-reclaim-installer.service, installed and enabled by the installer's finish.sh).
#
# Chat Computer Setup on Windows put the installer in a temporary FAT32 partition labelled CHATCOMP at the end
# of Chat Computer's space, marked chatcomputer/installer-<install id>, and its boot loader in \EFI\chatcomputer-setup
# on the EFI partition, started by a "Chat Computer setup" power-on entry. Now that Chat Computer runs from its own
# partition, this:
#   1. finds that partition: vfat, labelled CHATCOMP, with the mark for this install's id
#      (/etc/chatcomputer/install-id), and neither the partition Chat Computer runs from nor an EFI partition;
#   2. blanks the secrets it carried (Chat Computer's BitLocker key), in place, then deletes the partition;
#   3. if it sat right after Chat Computer's partition, grows that partition and its file system into the space;
#   4. deletes \EFI\chatcomputer-setup and the power-on entries that start it;
#   5. notes that it's done (/var/lib/chatcomputer/installer-reclaimed), so it never runs again.
# Nothing is deleted unless every check in step 1 holds. Tests point CHATCOMPUTER_SYSFS, CHATCOMPUTER_ESP,
# CHATCOMPUTER_STATE and CHATCOMPUTER_ETC at pretend folders and put stand-ins for the disk commands first on PATH.
set -u
SYS="${CHATCOMPUTER_SYSFS:-/sys}"
ESP="${CHATCOMPUTER_ESP:-/boot/efi}"
STATE="${CHATCOMPUTER_STATE:-/var/lib/chatcomputer}"
ETC="${CHATCOMPUTER_ETC:-/etc/chatcomputer}"
EFI_TYPE=c12a7328-f81f-11d2-ba4b-00a0c93ec93b

log() { echo "chatcomputer-reclaim: $*"; }

finished() {
  mkdir -p "$STATE"
  touch "$STATE/installer-reclaimed"
  exit 0
}

clear_loader() {
  # The installer's boot loader and the power-on entries that start it. Setup put the boot loader on the EFI
  # partition Windows starts from: usually the one Chat Computer uses too, but another disk's when Chat Computer has a
  # disk of its own, so the other EFI partitions are looked at as well.
  local dev mnt n
  if [ -d "$ESP/EFI/chatcomputer-setup" ]; then
    rm -rf "$ESP/EFI/chatcomputer-setup" && log "deleted \\EFI\\chatcomputer-setup"
  fi
  for dev in $(lsblk -rpno NAME,PARTTYPE 2>/dev/null | awk -v t="$EFI_TYPE" 'tolower($2) == t { print $1 }'); do
    findmnt -rno TARGET "$dev" >/dev/null 2>&1 && continue      # in use (the one above)
    mnt=$(mktemp -d) || continue
    if mount "$dev" "$mnt" 2>/dev/null; then
      if [ -d "$mnt/EFI/chatcomputer-setup" ]; then
        rm -rf "$mnt/EFI/chatcomputer-setup" && log "deleted \\EFI\\chatcomputer-setup on $dev"
      fi
      umount "$mnt" 2>/dev/null
    fi
    rmdir "$mnt" 2>/dev/null
  done
  for n in $(efibootmgr -v 2>/dev/null | awk 'tolower($0) ~ /\\efi\\chatcomputer-setup\\/ {
      n = substr($1, 5, 4); if (n ~ /^[0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f]$/) print n }'); do
    efibootmgr -q -b "$n" -B && log "deleted power-on entry Boot$n"
  done
}

disk_of() {
  # The disk a partition is on: sysfs keeps a partition inside its disk's folder.
  basename "$(readlink -f "$SYS/class/block/$1/..")"
}

find_installer() {
  local dev name mnt hit
  for dev in $(lsblk -rpno NAME,FSTYPE,LABEL 2>/dev/null | awk '$2 == "vfat" && $3 == "CHATCOMP" { print $1 }'); do
    name=${dev##*/}
    [ "$name" = "$root_name" ] && continue
    [ -e "$SYS/class/block/$name/partition" ] || continue
    [ "$(lsblk -rno PARTTYPE "$dev" 2>/dev/null)" = "$EFI_TYPE" ] && continue
    mnt=$(mktemp -d) || continue
    hit=1
    if mount -o ro "$dev" "$mnt" 2>/dev/null; then
      [ -f "$mnt/chatcomputer/installer-$id" ] && hit=0
      umount "$mnt" 2>/dev/null
    fi
    rmdir "$mnt" 2>/dev/null
    if [ "$hit" -eq 0 ]; then
      echo "$dev"
      return 0
    fi
  done
  return 1
}

blank_secrets() {
  # Overwrite the sign-in files where they lie (no truncating: FAT would give the old clusters back
  # still holding the data), then remove them.
  local mnt f size
  mnt=$(mktemp -d) || return 1
  if mount "$1" "$mnt" 2>/dev/null; then
    for f in "$mnt"/chatcomputer/secrets/*; do
      [ -f "$f" ] || continue
      size=$(stat -c %s "$f")
      [ "$size" -gt 0 ] && dd if=/dev/zero of="$f" bs="$size" count=1 conv=notrunc,fsync 2>/dev/null
      rm -f "$f"
    done
    sync
    umount "$mnt" 2>/dev/null
  fi
  rmdir "$mnt" 2>/dev/null
  return 0
}

# ------------------------------------------------------------------ 1. which partitions
id=$(cat "$ETC/install-id" 2>/dev/null) || finished
case "$id" in ''|*[!0-9a-f]*) log "no install id; nothing to do"; finished ;; esac

root_dev=$(findmnt -n -o SOURCE / 2>/dev/null || true)
root_name=${root_dev##*/}
if [ -z "$root_name" ] || [ ! -e "$SYS/class/block/$root_name/partition" ]; then
  log "Chat Computer doesn't run from a disk partition ($root_dev); only the boot loader is cleaned up"
  clear_loader
  finished
fi
root_disk=$(disk_of "$root_name")

inst_dev=$(find_installer) || {
  log "no installer partition is left"
  clear_loader
  finished
}
inst_name=${inst_dev##*/}
inst_disk=$(disk_of "$inst_name")
inst_num=$(cat "$SYS/class/block/$inst_name/partition")
inst_start=$(cat "$SYS/class/block/$inst_name/start")
log "the installer is $inst_dev (partition $inst_num of $inst_disk)"

# ------------------------------------------------------------------ 2. delete it
blank_secrets "$inst_dev"
wipefs -q -a "$inst_dev" >/dev/null 2>&1 || true
if ! sfdisk --no-reread --delete "/dev/$inst_disk" "$inst_num"; then
  log "couldn't delete the installer partition; trying again at the next start"
  exit 1
fi
partx -d --nr "$inst_num" "/dev/$inst_disk" 2>/dev/null || true
log "deleted the installer partition"

# ------------------------------------------------------------------ 3. grow into its space
if [ "$inst_disk" = "$root_disk" ]; then
  # sysfs counts in 512-byte sectors. Up to 2 MiB between them is alignment.
  root_end=$(( $(cat "$SYS/class/block/$root_name/start") + $(cat "$SYS/class/block/$root_name/size") ))
  gap=$(( inst_start - root_end ))
  if [ "$gap" -ge 0 ] && [ "$gap" -le 4096 ]; then
    root_num=$(cat "$SYS/class/block/$root_name/partition")
    if growpart "/dev/$root_disk" "$root_num"; then
      if resize2fs "$root_dev"; then log "Chat Computer's partition grew into the installer's space"; fi
    else
      log "Chat Computer's partition could not grow (growpart); the space stays free"
    fi
  else
    log "the installer wasn't right after Chat Computer's partition; its space stays free"
  fi
fi

# ------------------------------------------------------------------ 4. and 5.
clear_loader
finished
