#!/bin/bash
# The first-start clean-up (firstboot/reclaim-installer.sh) against a pretend disk: a fake /sys with the
# Windows disk's partitions (EFI, C:, Chat Computer, the installer right after it), folders standing in for the
# partitions' contents, and stand-ins for findmnt, lsblk, mount, sfdisk, partx, growpart, resize2fs,
# wipefs and efibootmgr that record what they were asked to do.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/.." && pwd)"   # linux/
FAILED=0
pass() { printf 'PASS %s\n' "$1"; }
fail() { printf 'FAIL %s\n' "$1"; FAILED=1; }
ID=0123456789abcdef01234567
RC="$REPO/firstboot/reclaim-installer.sh"

setup() {   # setup INSTALLER_START [MARK_ID] [INSTALLER_PARTTYPE] [ROOT_SOURCE]
  W="$(mktemp -d)"
  local inst_start=$1 mark=${2:-$ID} ptype=${3:-ebd0a0a2-b9e5-4433-87c0-68b6b72699c7} rootsrc=${4:-/dev/nvme0n1p6}
  mkdir -p "$W/bin" "$W/sys/class/block" "$W/esp/EFI/chatcomputer-setup" "$W/esp/EFI/ubuntu" "$W/state" "$W/etc"
  echo "$ID" > "$W/etc/install-id"
  echo shim > "$W/esp/EFI/chatcomputer-setup/shimx64.efi"
  echo shim > "$W/esp/EFI/ubuntu/shimx64.efi"
  part() {   # part NAME NUMBER START SIZE
    mkdir -p "$W/sys/devices/nvme0n1/$1"
    echo "$2" > "$W/sys/devices/nvme0n1/$1/partition"
    echo "$3" > "$W/sys/devices/nvme0n1/$1/start"
    echo "$4" > "$W/sys/devices/nvme0n1/$1/size"
    ln -s "../../devices/nvme0n1/$1" "$W/sys/class/block/$1"
  }
  part nvme0n1p1 1 2048 532480
  part nvme0n1p3 3 567296 33554432
  part nvme0n1p6 6 34121728 85000000
  part nvme0n1p5 5 "$inst_start" 12582912
  mkdir -p "$W/devs/nvme0n1p5/chatcomputer/secrets" "$W/devs/nvme0n1p1/EFI" "$W/devs/sda1/EFI/chatcomputer-setup" "$W/devs/sda1/EFI/Microsoft"
  echo shim > "$W/devs/sda1/EFI/chatcomputer-setup/shimx64.efi"
  echo "id=$mark" > "$W/devs/nvme0n1p5/chatcomputer/installer-$mark"
  printf '123456-234567-345678-456789-567890-678901-789012-890123\n' > "$W/devs/nvme0n1p5/chatcomputer/secrets/bitlocker-key"
  rec() { printf '#!/bin/bash\necho "%s $*" >> "%s/calls"\n%s\n' "$1" "$W" "${2:-}" > "$W/bin/$1"; chmod +x "$W/bin/$1"; }
  # findmnt: the root's source, and which EFI partition is in use (Chat Computer's, at /boot/efi).
  rec findmnt "case \"\$*\" in '-n -o SOURCE /') echo $rootsrc ;; *nvme0n1p1) echo /boot/efi ;; *) exit 1 ;; esac"
  # lsblk: partitions with file system and label, with partition type, or one partition's type. sda1 is
  # another disk's EFI partition (Windows' when Chat Computer has a disk of its own).
  rec lsblk "case \"\$2\" in \
NAME,FSTYPE,LABEL) printf '/dev/nvme0n1p1 vfat SYSTEM\n/dev/nvme0n1p3 ntfs Windows\n/dev/nvme0n1p5 vfat CHATCOMP\n/dev/nvme0n1p6 ext4 \n/dev/sda1 vfat \n' ;; \
NAME,PARTTYPE) printf '/dev/nvme0n1p1 c12a7328-f81f-11d2-ba4b-00a0c93ec93b\n/dev/nvme0n1p3 ebd0a0a2-b9e5-4433-87c0-68b6b72699c7\n/dev/nvme0n1p5 $ptype\n/dev/nvme0n1p6 0fc63daf-8483-4772-8e79-3d69d8477de4\n/dev/sda1 C12A7328-F81F-11D2-BA4B-00A0C93EC93B\n' ;; \
PARTTYPE) case \"\$3\" in /dev/nvme0n1p5) echo $ptype ;; /dev/nvme0n1p1|/dev/sda1) echo c12a7328-f81f-11d2-ba4b-00a0c93ec93b ;; esac ;; esac"
  rec mount "dir=\"\${@: -1}\"; dev=\"\${@: -2:1}\"; rmdir \"\$dir\" && ln -s \"$W/devs/\${dev##*/}\" \"\$dir\""
  rec umount "rm -f \"\$1\" && mkdir \"\$1\""
  rec dd "exec /bin/dd \"\$@\""
  for c in wipefs sfdisk partx growpart resize2fs; do rec "$c"; done
  rec efibootmgr "if [ \"\$1\" = -v ]; then printf 'BootCurrent: 0001\nBoot0000* Windows Boot Manager\tHD(1,GPT,x,0x800,0x82000)/File(\\\\EFI\\\\Microsoft\\\\Boot\\\\bootmgfw.efi)\nBoot0001* ubuntu\tHD(1,GPT,x,0x800,0x82000)/File(\\\\EFI\\\\ubuntu\\\\shimx64.efi)\nBoot0003* Chat Computer setup\tHD(1,GPT,x,0x800,0x82000)/File(\\\\EFI\\\\chatcomputer-setup\\\\shimx64.efi)\n'; fi"
}

run() {
  env PATH="$W/bin:$PATH" CHATCOMPUTER_SYSFS="$W/sys" CHATCOMPUTER_ESP="$W/esp" CHATCOMPUTER_STATE="$W/state" CHATCOMPUTER_ETC="$W/etc" \
    bash "$RC" > "$W/out" 2>&1
  echo $?
}

# 1. The usual case: the installer right after Chat Computer's partition.
setup $((34121728 + 85000000))
rc=$(run)
[ "$rc" = 0 ] && pass "exits 0" || { fail "exit status $rc"; cat "$W/out"; }
grep -q "^sfdisk --no-reread --delete /dev/nvme0n1 5$" "$W/calls" && pass "deletes partition 5, the installer" || fail "sfdisk call"
! grep -q "delete /dev/nvme0n1 [1-46]" "$W/calls" && pass "deletes nothing else" || fail "deleted another partition"
grep -q "^growpart /dev/nvme0n1 6$" "$W/calls" && grep -q "^resize2fs /dev/nvme0n1p6$" "$W/calls" \
  && pass "grows Chat Computer's partition and file system into the space" || fail "not grown"
grep -q "of=.*bitlocker-key.*conv=notrunc" "$W/calls" && [ ! -e "$W/devs/nvme0n1p5/chatcomputer/secrets/bitlocker-key" ] \
  && pass "blanks the secret in place, then removes it" || fail "secret not blanked"
[ ! -d "$W/esp/EFI/chatcomputer-setup" ] && [ -d "$W/esp/EFI/ubuntu" ] \
  && pass "removes the installer's boot loader, and only that" || fail "boot loader files"
[ ! -d "$W/devs/sda1/EFI/chatcomputer-setup" ] && [ -d "$W/devs/sda1/EFI/Microsoft" ] && ! grep -q "^mount .*nvme0n1p1" "$W/calls" \
  && pass "...also from another disk's EFI partition, leaving the one in use to the first step" || fail "other EFI partition"
grep -q "^efibootmgr -q -b 0003 -B$" "$W/calls" && ! grep -q "efibootmgr -q -b 000[01]" "$W/calls" \
  && pass "removes the \"Chat Computer setup\" power-on entry, and only that" || fail "power-on entries"
[ -f "$W/state/installer-reclaimed" ] && pass "notes that it's done" || fail "no done mark"
rm -rf "$W"

# 2. A CHATCOMP partition from another install (another id): left alone.
setup $((34121728 + 85000000)) fedcba9876543210fedcba98
rc=$(run)
{ [ "$rc" = 0 ] && ! grep -q "^sfdisk" "$W/calls" && [ -e "$W/devs/nvme0n1p5/chatcomputer/secrets/bitlocker-key" ]; } \
  && pass "leaves a partition without this install's mark alone" || fail "touched another install's partition"
rm -rf "$W"

# 3. Not right after Chat Computer's partition: deleted, but nothing grows.
setup $((34121728 + 85000000 + 2000000))
rc=$(run)
{ [ "$rc" = 0 ] && grep -q "^sfdisk --no-reread --delete /dev/nvme0n1 5$" "$W/calls" && ! grep -q "^growpart" "$W/calls"; } \
  && pass "doesn't grow Chat Computer's partition across other space" || fail "grew across a gap"
rm -rf "$W"

# 4. A CHATCOMP-labelled EFI partition: never deleted.
setup $((34121728 + 85000000)) "$ID" c12a7328-f81f-11d2-ba4b-00a0c93ec93b
rc=$(run)
{ [ "$rc" = 0 ] && ! grep -q "^sfdisk" "$W/calls"; } && pass "never deletes an EFI partition" || fail "deleted an EFI partition"
rm -rf "$W"

# 5. Deleting fails: no done mark, so it tries again at the next start.
setup $((34121728 + 85000000))
printf '#!/bin/bash\necho "sfdisk $*" >> "%s/calls"\nexit 1\n' "$W" > "$W/bin/sfdisk"
rc=$(run)
{ [ "$rc" = 1 ] && [ ! -f "$W/state/installer-reclaimed" ] && ! grep -q "^growpart" "$W/calls"; } \
  && pass "tries again at the next start if deleting fails" || fail "failure handling (rc $rc)"
rm -rf "$W"

exit "$FAILED"
