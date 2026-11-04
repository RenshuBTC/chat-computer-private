#!/bin/bash
# Chat Computer Setup for Mac (mac/setup.sh) on one of GitHub's Macs. They are virtual Macs with Apple silicon, so
# Chat Computer can't really be installed there (Asahi Linux needs a real Mac), but they have macOS's own tools: each
# part of setup that uses them runs here against real macOS. Nothing touches this Mac's own disk: a disk image
# stands in for it.
#
#   /bin/bash tests/mac_ci/run.sh RESULTS_DIR     (run by .github/workflows/mac.yml; macOS's bash 3.2)
#
#  1. Setup as a person starts it, plan only: its checks, this Mac's disk read with diskutil (kept in RESULTS_DIR)
#     and the folders' sizes. GitHub's Macs have little free space, so it may stop for lack of space: that's fine.
#  2. A disk image laid out the way the Asahi installer leaves a Mac, made with the installer's own commands:
#     setup finds Chat Computer's pieces on it, makes the Chat Computer drive and copies folders onto it, writes Chat Computer's
#     first start to its EFI partition, and then removes Chat Computer again.
#  3. The real downloads: the Asahi installer and Fedora's image, checked against their fingerprints, Chat Computer's
#     icon added with the installer's own Python; then a second run, which mustn't download Fedora again.
#  4. The real Asahi installer, started by setup and answered by drive-installer.exp, until it turns down a
#     virtual Mac (as it does before it changes anything).
# summary.txt in RESULTS_DIR has a PASS or FAIL line for each check. Exit status 1 if any failed.
# (Variables set here are read by setup's parts; output of commands run with sudo goes to these results on purpose.)
# shellcheck disable=SC2024,SC2034
set -u
REPO="$(cd "$(dirname "$0")/../.." && pwd)"
OUT="${1:?usage: run.sh RESULTS_DIR}"
mkdir -p "$OUT"
OUT="$(cd "$OUT" && pwd)"
SUM="$OUT/summary.txt"
: > "$SUM"
FAILED=0
pass() { printf 'PASS %s\n' "$1" | tee -a "$SUM"; }
fail() { printf 'FAIL %s\n' "$1" | tee -a "$SUM"; FAILED=1; }
note() { printf '     %s\n' "$1" | tee -a "$SUM"; }
ok() {   # ok WHAT COMMAND...: PASS if the command succeeds
  local what="$1"
  shift
  if "$@" >/dev/null 2>&1; then pass "$what"; else fail "$what"; fi
}
section() { printf '\n== %s\n' "$1" | tee -a "$SUM"; }
PY="$(command -v python3)"
T="$(mktemp -d /tmp/chatcomputer-ci.XXXXXX)"

{ sw_vers; uname -m; sysctl -n machdep.cpu.brand_string; echo "virtual: $(sysctl -n kern.hv_vmm_present)"
  /bin/bash --version | head -n 1; rsync --version 2>&1 | head -n 1; tar --version; expect -v; df -h /; id
} > "$OUT/mac.txt" 2>&1
note "$(sw_vers -productVersion) on $(sysctl -n machdep.cpu.brand_string), $(/bin/bash --version | head -n 1)"

# A person's folders: Finder's .DS_Store files, a Photos library (not copied), files with extended attributes
# (which exFAT and FAT would keep as ._ files if they were copied), something big.
H="$T/home"
mkdir -p "$H/Desktop" "$H/Documents/Projects" "$H/Pictures/Photos Library.photoslibrary/originals"
printf 'buy milk\n' > "$H/Desktop/todo.txt"
printf '\0finder' > "$H/Desktop/.DS_Store"
printf 'hello from macOS\n' > "$H/Documents/Projects/notes.txt"
xattr -w com.apple.quarantine "0081;00000000;Safari;" "$H/Documents/Projects/notes.txt"
xattr -w com.example.chatcomputer "an extended attribute" "$H/Desktop/todo.txt"
head -c 3000000 /dev/urandom > "$H/Pictures/beach.jpg"
printf 'h' > "$H/Pictures/Photos Library.photoslibrary/originals/IMG_1.heic"
# The downloaded Chat Computer folder carries the download's quarantine mark (on a GitHub Mac it doesn't by itself).
xattr -w com.apple.quarantine "0081;00000000;Safari;" "$REPO/mac/esp/firstboot.sh"

# ------------------------------------------------------------------ 1. plan only, as a person starts it
section "1. Setup, plan only, on this Mac"
W1="$T/work-plan"
printf '1\ny\n' | CHATCOMPUTER_PLAN_ONLY=1 CHATCOMPUTER_MAC_ROOT="$H" CHATCOMPUTER_SETUP_WORK="$W1" /bin/bash "$REPO/mac/setup.sh" \
  > "$OUT/1-plan.txt" 2>&1
rc=$?
for f in "$W1"/*.plist "$W1/setup.log"; do [ -f "$f" ] && cp "$f" "$OUT/1-$(basename "$f")"; done
ok "checks accept this (virtual) Apple silicon Mac in plan-only mode" grep -q "on an Apple M" "$OUT/1-plan.txt"
if grep -q "already on this Mac" "$OUT/1-plan.txt"; then fail "finds no Chat Computer on this Mac's disk"; else pass "finds no Chat Computer on this Mac's disk"; fi
ok "folder sizes from du -A" grep -Eq "Pictures +[0-9.]+ MB" "$OUT/1-plan.txt"
line="$(grep -E 'disk: container=disk[0-9]+ store=disk[0-9]+s[0-9]+ total=[0-9]+ free=[0-9]+ preferred-min=[0-9]+ available=[0-9]+' "$W1/setup.log" 2>/dev/null)"
if [ -n "$line" ]; then pass "reads macOS's disk with diskutil and plutil"; note "${line#*  }"; else fail "reads macOS's disk with diskutil and plutil"; fi
if [ "$rc" = 0 ] && grep -q "Nothing was changed" "$OUT/1-plan.txt"; then
  pass "plan made"
  note "$(grep '^PLAN ' "$W1/setup.log")"
elif [ "$rc" = 1 ] && grep -q "Chat Computer needs" "$OUT/1-plan.txt"; then
  pass "plan stops for lack of space (GitHub's Macs have little), before changing anything"
else
  fail "plan (exit status $rc; see 1-plan.txt)"
fi

# ------------------------------------------------------------------ 2. on a disk image
section "2. On a disk image laid out like a Mac after the Asahi installer"
export CHATCOMPUTER_MAC_ROOT="$H" CHATCOMPUTER_SETUP_WORK="$T/work"
# shellcheck source=../../mac/setup.sh
. "$REPO/mac/setup.sh"          # its parts; it doesn't start when sourced
mkdir -p "$WORK"

parts() {   # parts DISK: "identifier|content|volume name" per partition, in order
  local i=0 id
  diskutil list -plist "$1" > "$T/list.plist" 2>/dev/null || return 1
  while id="$(plist "$T/list.plist" "AllDisksAndPartitions.0.Partitions.$i.DeviceIdentifier")" && [ -n "$id" ]; do
    printf '%s|%s|%s\n' "$id" "$(plist "$T/list.plist" "AllDisksAndPartitions.0.Partitions.$i.Content")" \
      "$(plist "$T/list.plist" "AllDisksAndPartitions.0.Partitions.$i.VolumeName")"
    i=$((i + 1))
  done
}
after() { parts "$D" | awk -F'|' -v p="$1" 'found { print $1; exit } $1 == p { found = 1 }'; }
info() { diskutil info -plist "$1" > "$T/info.plist" 2>/dev/null; plist "$T/info.plist" "$2"; }

IMG="$T/mac"
D=""
if hdiutil create -quiet -size 6g -type SPARSE -layout GPTSPUD -fs APFS -volname "Macintosh HD" "$IMG" \
   && D="$(hdiutil attach -nomount -noverify -noautofsck "$IMG.sparseimage" | awk 'NR == 1 { sub("/dev/", "", $1); print $1 }')" \
   && [ -n "$D" ]; then
  pass "disk image attached ($D)"
else
  fail "disk image attached"
fi
MACPART="$(parts "$D" | awk -F'|' '$2 == "Apple_APFS" { print $1; exit }')"
# As the Asahi installer does it: macOS's container shrinks, then (main.py, osinstall.py) the start-up volume's
# container (2.5 GB there), the EFI partition (FAT32, named "EFI - " and the OS's name, 11 letters), Boot, Root.
{
  sudo diskutil apfs resizeContainer "$MACPART" 1.5G
  sudo diskutil addPartition "$MACPART" APFS "Chat Computer" 700M
} > "$OUT/2-layout.txt" 2>&1
STUBPART="$(after "$MACPART")"
sudo diskutil addPartition "$STUBPART" %EFI% %noformat% 500M >> "$OUT/2-layout.txt" 2>&1
ESP="$(after "$STUBPART")"
sudo newfs_msdos -F 32 -v "EFI - Chat" "/dev/r$ESP" >> "$OUT/2-layout.txt" 2>&1
sudo diskutil addPartition "$ESP" %Linux% %noformat% 200M >> "$OUT/2-layout.txt" 2>&1
BOOTP="$(after "$ESP")"
sudo diskutil addPartition "$BOOTP" %Linux% %noformat% 600M >> "$OUT/2-layout.txt" 2>&1
ROOTP="$(after "$BOOTP")"
diskutil mount "$ESP" >> "$OUT/2-layout.txt" 2>&1
ESPMNT="$(info "$ESP" MountPoint)"
FEDORA_GRUB="search --fs-uuid --set=root $FEDORA_BOOT_UUID
set prefix=(\$root)/grub2
configfile (\$root)/grub2/grub.cfg"
if [ -n "$ESPMNT" ] && sudo mkdir -p "$ESPMNT/EFI/fedora" \
   && printf '%s\n' "$FEDORA_GRUB" | sudo tee "$ESPMNT/EFI/fedora/grub.cfg" > /dev/null; then
  pass "the image has macOS's container, Chat Computer's start-up volume, EFI ($ESP), Boot and Root, like the installer makes them"
else
  fail "the image laid out like the installer leaves a Mac (see 2-layout.txt)"
fi
diskutil unmount "$ESP" >> "$OUT/2-layout.txt" 2>&1
parts "$D" > "$OUT/2-parts-installed.txt"
diskutil list "$D" >> "$OUT/2-parts-installed.txt" 2>&1
cp "$T/list.plist" "$OUT/2-list-installed.plist"

find_install "$D"
cp "$WORK/apfs.plist" "$OUT/2-apfs-installed.plist" 2>/dev/null
note "found: start-up volume $FOUND_STUB ($FOUND_STUB_VOLUME), EFI $FOUND_ESP, Boot $FOUND_BOOT, Linux $FOUND_LINUX, drive ${FOUND_DRIVE:-none}"
if [ -n "$STUBPART" ] && [ "$FOUND_STUB:$FOUND_ESP:$FOUND_BOOT:$FOUND_LINUX" = "$STUBPART:$ESP:$BOOTP:$ROOTP" ] \
   && [ -z "$FOUND_DRIVE" ] && [ -n "$FOUND_STUB_VOLUME" ]; then
  pass "find_install finds each of Chat Computer's pieces in diskutil's real output"
else
  fail "find_install (wanted $STUBPART $ESP $BOOTP $ROOTP)"
fi
# This Mac's own system volume, as setup blesses it to make macOS the start-up disk again (not the snapshot
# / is mounted from).
mv_="$(macos_volume)"
mv_name="$(info "$mv_" VolumeName)"
note "macOS's system volume: $mv_ ($mv_name); / is on $(plist "$WORK/root.plist" DeviceIdentifier)"
if [[ "$mv_" =~ ^disk[0-9]+s[0-9]+$ ]] && [ -n "$mv_name" ]; then pass "finds macOS's own system volume"; else fail "macOS's system volume ($mv_)"; fi

# The Chat Computer drive, in the space left over, and the folders copied onto it.
(
  ROOT_DEV="$ROOTP" DISK="$D" DRIVE_BYTES=$((2 * GB))
  make_drive
  printf 'FOUND_DRIVE=%s\nDRIVE_MOUNT=%q\nDRIVE_PARTUUID=%s\n' "$FOUND_DRIVE" "$DRIVE_MOUNT" "$DRIVE_PARTUUID" > "$T/drive.vars"
) > "$OUT/2-make-drive.txt" 2>&1
FOUND_DRIVE="" DRIVE_MOUNT="" DRIVE_PARTUUID=""
[ -f "$T/drive.vars" ] && . "$T/drive.vars"
if [ -n "$FOUND_DRIVE" ] && [ -d "$DRIVE_MOUNT" ] && [ -d "$DRIVE_MOUNT/.chatcomputer" ]; then
  pass "make_drive makes the Chat Computer drive ($FOUND_DRIVE, at $DRIVE_MOUNT)"
else
  fail "make_drive (see 2-make-drive.txt)"
fi
ok "the drive is exFAT" test "$(info "$FOUND_DRIVE" FilesystemType)" = exfat
diskutil info -plist "$FOUND_DRIVE" > "$OUT/2-drive-info.plist" 2>/dev/null
# Chat Computer mounts the drive by its partition's GPT id (fstab PARTUUID=); setup takes it from diskutil's DiskUUID.
if sudo "$PY" - "/dev/r$D" "$DRIVE_PARTUUID" <<'EOF' >> "$OUT/2-make-drive.txt" 2>&1
import struct, sys, uuid
dev, want = sys.argv[1], sys.argv[2].upper()
with open(dev, "rb") as f:
    head = f.read(512 * 64)
lba, count, size = struct.unpack_from("<QII", head, 512 + 72)
for i in range(count):
    e = head[lba * 512 + i * size: lba * 512 + (i + 1) * size]
    if len(e) == size and str(uuid.UUID(bytes_le=e[16:32])).upper() == want:
        print("GPT entry", i + 1, "has the id", want)
        sys.exit(0)
print("no GPT entry has the id", want)
sys.exit(1)
EOF
then
  pass "diskutil's DiskUUID is the partition's GPT id (what Chat Computer's fstab uses)"
else
  fail "diskutil's DiskUUID ($DRIVE_PARTUUID) is the partition's GPT id"
fi

( copy_folders "$DRIVE_MOUNT" Desktop Documents Pictures ) > "$OUT/2-copy.txt" 2>&1
ok "folders copied" test "$(cat "$DRIVE_MOUNT/Desktop/todo.txt" 2>/dev/null)" = "buy milk"
ok "big files copied whole" cmp -s "$H/Pictures/beach.jpg" "$DRIVE_MOUNT/Pictures/beach.jpg"
ok "no .DS_Store or Photos library copied" test ! -e "$DRIVE_MOUNT/Desktop/.DS_Store" -a ! -e "$DRIVE_MOUNT/Pictures/Photos Library.photoslibrary"
dot="$(find "$DRIVE_MOUNT" -name '._*' 2>/dev/null | head -n 5)"
if [ -z "$dot" ]; then pass "no ._ files on the drive (extended attributes not copied)"; else fail "._ files on the drive: $dot"; fi

# Chat Computer's first start, on its EFI partition.
(
  EFI_DEV="$ESP" ACCOUNT="sammy" REAL_NAME='Sam "Lee"' RECOVERY="abcd-efgh-jkmn-pqrs" TZONE="Asia/Singapore"
  INSTALL_ID="0123456789abcdef01234567" WIFI_SSID='Home "5G"' WIFI_PSK="correct horse"
  write_first_start
) > "$OUT/2-first-start.txt" 2>&1
rc=$?
ok "write_first_start finishes" test "$rc" = 0
diskutil mount "$ESP" > /dev/null 2>&1
E="$(info "$ESP" MountPoint)"
ok "first start's program on the EFI partition" cmp -s "$REPO/mac/esp/firstboot.sh" "$E/chatcomputer/firstboot.sh"
ok "Fedora's own start kept, to put back" test "$(cat "$E/chatcomputer/fedora-grub.cfg" 2>/dev/null)" = "$FEDORA_GRUB"
g="$E/EFI/fedora/grub.cfg"
if grep -q "vmlinuz-$FEDORA_KERNEL" "$g" && grep -q "root=UUID=$FEDORA_ROOT_UUID" "$g" && ! grep -q @ "$g" \
   && grep -qF '"systemd.run=/bin/sh /boot/efi/chatcomputer/firstboot.sh" systemd.unit=kernel-command-line.target' "$g"; then
  pass "GRUB starts the first start once (kernel, root and program filled in)"
else
  fail "the first start's GRUB file"
fi
cp "$g" "$OUT/2-grub.cfg" 2>/dev/null
if "$PY" - "$E/chatcomputer" "$DRIVE_PARTUUID" <<'EOF' > "$OUT/2-first-start-check.txt" 2>&1
import base64, json, sys, tarfile
p, drive = sys.argv[1], sys.argv[2]
env = dict(line.split("=", 1) for line in open(p + "/setup.env").read().splitlines())
v = {k: base64.b64decode(x).decode() for k, x in env.items()}
assert v["USER_NAME"] == "sammy" and v["REAL_NAME"] == 'Sam "Lee"', v
assert v["WIFI_SSID"] == 'Home "5G"' and v["WIFI_PSK"] == "correct horse" and v["TIMEZONE"] == "Asia/Singapore"
assert v["MAC_FILES_PARTUUID"].upper() == drive.upper() and v["RECOVERY_PASSWORD"] == "abcd-efgh-jkmn-pqrs"
import os
assert os.listdir(p + "/secrets") == [], os.listdir(p + "/secrets")
assert json.load(open(p + "/settings.json"))["user_name"] == 'Sam "Lee"'
with tarfile.open(p + "/app.tar.gz") as t:
    members = t.getmembers()
names = [m.name for m in members]
assert "chatcomputer/__init__.py" in names and "linux/install-chatcomputer.sh" in names, names[:20]
bad = [n for n in names if n.split("/")[-1].startswith("._") or "__pycache__" in n or n.startswith(("mac/", "windows/"))]
assert not bad, bad
xattrs = sorted({k for m in members for k in m.pax_headers if "xattr" in k.lower() or "acl" in k.lower()})
assert not xattrs, xattrs
print("setup.env, settings and Chat Computer's package as Chat Computer's first start reads them;", len(names), "files")
EOF
then
  pass "settings and Chat Computer's package read back as the first start reads them (no ._ or xattr entries)"
else
  fail "what's on the EFI partition (see 2-first-start-check.txt)"
fi
dot="$(find "$E" -name '._*' 2>/dev/null | head -n 5)"
if [ -z "$dot" ]; then pass "no ._ files on the EFI partition"; else fail "._ files on the EFI partition: $dot"; fi
ls -laR "$E" > "$OUT/2-esp-files.txt" 2>&1
diskutil unmount "$ESP" > /dev/null 2>&1

# Removing Chat Computer: the image stands in for macOS's disk (setup's read_disk would read this Mac's own).
read_apfs
c="$(container_on "$MACPART")"
IMGCONT="$(plist "$WORK/apfs.plist" "Containers.${c:-x}.ContainerReference")"
MACVOL="$(system_volume "${c:-x}")"
note "the image's macOS: container $IMGCONT on $MACPART, volume $MACVOL"
read_disk() {
  CONTAINER="$IMGCONT" STORE="$MACPART" DISK="$D"
}
macos_volume() { printf '%s' "$MACVOL"; }
mkdir -p "$H/Documents"
printf 'note\n' > "$H/Documents/Chat Computer recovery password.txt"
# Remove; copy the drive's files to macOS first; REMOVE; the Mac's password (none here: bless only warns).
printf 'y\nREMOVE\n\n' | ( remove ) > "$OUT/2-remove.txt" 2>&1
rc=$?
ok "remove finishes" test "$rc" = 0
ok "it says Chat Computer is deleted and macOS has its space back" grep -q "macOS has its space back" "$OUT/2-remove.txt"
ok "it blesses the image's macOS volume" grep -q "Your Mac's password, to make macOS the start-up disk" "$OUT/2-remove.txt"
parts "$D" > "$OUT/2-parts-removed.txt"
diskutil list "$D" >> "$OUT/2-parts-removed.txt" 2>&1
left="$(parts "$D" | awk -F'|' '{ print $2 }' | tr '\n' ' ')"
if [ "$left" = "EFI Apple_APFS " ]; then pass "only macOS's partitions are left"; else fail "partitions left: $left"; fi
cap="$(diskutil apfs list -plist "$IMGCONT" > "$T/c.plist" 2>/dev/null; plist "$T/c.plist" Containers.0.CapacityCeiling)"
note "macOS's container: $(size_text "${cap:-0}") (was 1.5 GB)"
ok "macOS's container grew into the space" test "${cap:-0}" -gt 5000000000
ok "the drive's files were copied to \"Chat Computer files\" first" test "$(cat "$H/Chat Computer files/Desktop/todo.txt" 2>/dev/null)" = "buy milk"
ok "the recovery note is gone" test ! -e "$H/Documents/Chat Computer recovery password.txt"
# Space left after macOS's partition (a removal whose last step failed) goes back to macOS on the next run.
sudo diskutil apfs resizeContainer "$MACPART" 1.5G > "$OUT/2-grow.txt" 2>&1
printf 'y\n' | ( remove ) >> "$OUT/2-grow.txt" 2>&1
cap="$(diskutil apfs list -plist "$IMGCONT" > "$T/c.plist" 2>/dev/null; plist "$T/c.plist" Containers.0.CapacityCeiling)"
if grep -q "free space is right after macOS's partition" "$OUT/2-grow.txt" && [ "${cap:-0}" -gt 5000000000 ]; then
  pass "a later run gives macOS the free space after its partition"
else
  fail "a later run gives macOS the free space after its partition (see 2-grow.txt)"
fi
hdiutil detach -force "$D" > /dev/null 2>&1
rm -f "$IMG.sparseimage"

# ------------------------------------------------------------------ 3. the real downloads
section "3. Downloads: the Asahi installer and Fedora's image"
( prepare_installer; printf 'INST=%q\n' "$INST" > "$T/inst.vars" ) > "$OUT/3-download.txt" 2>&1
rc=$?
ok "prepare_installer finishes (both fingerprints match)" test "$rc" = 0
INST=""
[ -f "$T/inst.vars" ] && . "$T/inst.vars"
pkg="$WORK/os/$FEDORA_PACKAGE"
if "$PY" - "$pkg" "$REPO/mac/chatcomputer.icns" <<'EOF' >> "$OUT/3-download.txt" 2>&1
import sys, zipfile
z = zipfile.ZipFile(sys.argv[1])
names = set(z.namelist())
assert {"boot.img", "root.img", "chatcomputer.icns"} <= names and any(n.startswith("esp/") for n in names), sorted(names)[:20]
assert z.read("chatcomputer.icns") == open(sys.argv[2], "rb").read()
for n in names:
    if n.startswith("esp/") and not n.endswith("/"):
        z.read(n)          # checks each small file's CRC
print("package:", len(names), "files, with Chat Computer's icon")
EOF
then
  pass "Chat Computer's icon is in Fedora's package (added with the installer's Python), and the package still reads"
else
  fail "Fedora's package with Chat Computer's icon (see 3-download.txt)"
fi
ok "the installer gets Chat Computer's installer_data.json" cmp -s "$REPO/mac/installer_data.json" "$INST/installer_data.json"
ok "the new fingerprint is noted" test -s "$pkg.with-icon"
( prepare_installer ) > "$OUT/3-download-again.txt" 2>&1
ok "a second run doesn't download Fedora again" grep -q "downloaded already" "$OUT/3-download-again.txt"

# ------------------------------------------------------------------ 4. the real installer's start
section "4. The Asahi installer, answered by drive-installer.exp"
( MACOS_NEW=$((100 * GB)); CHATCOMPUTER_BYTES=$((60 * GB)); run_installer ) > "$OUT/4-installer.txt" 2>&1
rc=$?
[ -f "$WORK/asahi-installer.log" ] && cp "$WORK/asahi-installer.log" "$OUT/4-asahi-installer.log"
ok "the installer starts as Chat Computer's" grep -q "Welcome to the Chat Computer installer" "$OUT/4-installer.txt"
ok "setup's answer reached it (past \"Press enter to continue.\")" grep -q "Collecting system information" "$OUT/4-installer.txt"
if grep -q "not supported" "$OUT/4-installer.txt" && [ "$rc" = 1 ] && grep -q "The Asahi installer didn't finish" "$OUT/4-installer.txt"; then
  pass "it turns down the virtual Mac, and setup stops and says so"
else
  fail "the installer's end on a virtual Mac (exit status $rc; see 4-installer.txt)"
fi

cp "$WORK/setup.log" "$OUT/setup.log" 2>/dev/null
section "Summary"
printf '%s checks passed, %s failed\n' "$(grep -c '^PASS' "$SUM")" "$(grep -c '^FAIL' "$SUM")" | tee -a "$SUM"
rm -rf "$T"
exit "$FAILED"
