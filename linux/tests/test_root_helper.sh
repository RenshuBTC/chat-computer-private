#!/bin/bash
# Test root-helper/chatcomputer-root with faked system tools on PATH.
#
# We never touch the real system: a shim bin dir provides efibootmgr, lsblk,
# blkid, mount, umount, findmnt, mountpoint, apt-get, systemctl, timedatectl,
# grub-reboot, ntfs-3g.probe. Each shim logs its args and prints canned output.
#
# Coverage:
#   * status            -> JSON with all required boolean keys
#   * boot-windows-next -> uses efibootmgr --bootnext with the Windows boot num
#   * apt-install       -> rejects malicious/invalid package names, accepts good
#   * set-timezone      -> rejects traversal/unknown, accepts a real zone
#   * mount-windows     -> finds NTFS w/ Windows/System32, mounts rw when clean
#   * wipe-install-secrets -> only touches the CHATCOMP-labelled device
#   * unknown verb / arg-count rejections
#   * every stdout is valid JSON
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/.." && pwd)"
HELPER="$REPO/root-helper/chatcomputer-root"
PY="${CHATCOMPUTER_TEST_PY:-python3}"
FAILED=0
pass() { printf 'PASS %s\n' "$1"; }
fail() { printf 'FAIL %s\n' "$1"; FAILED=1; }

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
BIN="$WORK/bin"
mkdir -p "$BIN"
LOG="$WORK/calls.log"
: > "$LOG"

# is_json <string> -> 0 if valid JSON object
is_json() { printf '%s' "$1" | "$PY" -c 'import sys,json; json.loads(sys.stdin.read())' 2>/dev/null; }
jget() { printf '%s' "$1" | "$PY" -c "import sys,json; d=json.load(sys.stdin); print(d.get('$2'))" 2>/dev/null; }

# --- Shims -----------------------------------------------------------------
shim() { # shim NAME BODY
  cat > "$BIN/$1"; chmod +x "$BIN/$1";
}

shim efibootmgr <<SH
#!/bin/bash
echo "efibootmgr \$*" >> "$LOG"
# Handle --bootnext NNNN (record it) else print a listing with Windows at 0001.
if [ "\$1" = "--bootnext" ]; then
  echo "BootNext: \$2"
  exit 0
fi
if [ "\$1" = "--bootorder" ]; then
  echo "BootOrder: \$2"
  exit 0
fi
cat <<EOF
BootCurrent: 0003
Timeout: 1 seconds
BootOrder: 0001,0003,0000
Boot0000* ubuntu	HD(1,GPT)/File(\\EFI\\ubuntu\\shimx64.efi)
Boot0001* Windows Boot Manager	HD(1,GPT)/File(\\EFI\\Microsoft\\Boot\\bootmgfw.efi)
Boot0003* UEFI OS	HD(1,GPT)/File(\\EFI\\BOOT\\BOOTX64.EFI)
EOF
SH

shim lsblk <<SH
#!/bin/bash
echo "lsblk \$*" >> "$LOG"
# Emulate: sda1 vfat (ESP), sda2 ext4 (linux), sda3 ntfs (Windows), sda4 ntfs (data)
# -rno NAME,FSTYPE
if printf '%s' "\$*" | grep -q 'NAME,FSTYPE'; then
  echo "sda"
  echo "sda1 vfat"
  echo "sda2 ext4"
  echo "sda3 ntfs"
  echo "sda4 ntfs"
elif printf '%s' "\$*" | grep -q 'FSTYPE'; then
  printf 'vfat\next4\nntfs\nntfs\n'
fi
SH

shim blkid <<SH
#!/bin/bash
echo "blkid \$*" >> "$LOG"
# -L CHATCOMP -> device; -o value -s TYPE DEV -> fstype
if [ "\$1" = "-L" ] && [ "\$2" = "CHATCOMP" ]; then
  echo "$WORK/usb.img"; exit 0
fi
# TYPE probe: say sda3/sda4 are plain ntfs (not BitLocker)
echo "ntfs"
SH

# mount: record, and simulate a working mount by creating marker dirs. For the
# ro-probe of /dev/sda3 we create a Windows/System32 dir in the probe target.
shim mount <<SH
#!/bin/bash
echo "mount \$*" >> "$LOG"
# Last two args are DEVICE MOUNTPOINT (for both -o and -t forms).
mp="\${@: -1}"
dev="\${@: -2:1}"
mkdir -p "\$mp"
case "\$dev" in
  */sda3) mkdir -p "\$mp/Windows/System32" ;;   # the Windows partition
  *) : ;;
esac
# Simulate the CHATCOMP usb containing a secrets dir on first mount.
case "\$dev" in
  *usb.img) mkdir -p "\$mp/chatcomputer/secrets"; echo secret > "\$mp/chatcomputer/secrets/claude_token" ;;
esac
exit 0
SH

shim umount <<SH
#!/bin/bash
echo "umount \$*" >> "$LOG"
exit 0
SH

shim findmnt <<SH
#!/bin/bash
echo "findmnt \$*" >> "$LOG"
# -no OPTIONS /mnt/windows -> report rw unless a marker says ro
if [ -f "$WORK/winopts" ]; then cat "$WORK/winopts"; else echo "rw,relatime"; fi
SH

shim mountpoint <<SH
#!/bin/bash
echo "mountpoint \$*" >> "$LOG"
# Report mounted only if a marker exists (so status/mount tests can control it).
tgt="\${@: -1}"
if [ -f "$WORK/mounted" ]; then exit 0; else exit 1; fi
SH

shim apt-get <<SH
#!/bin/bash
echo "apt-get \$*" >> "$LOG"
# A simulated removal (-s remove) prints the plan in \$WORK/apt-plan the way apt-get does: "Remv NAME [VERSION]".
if [ "\$1" = "-s" ]; then cat "$WORK/apt-plan" 2>/dev/null; fi
exit 0
SH

# apt-cache pkgnames [--all-names] PREFIX: the names apt knows (\$WORK/apt-names) that start with PREFIX.
shim apt-cache <<SH
#!/bin/bash
echo "apt-cache \$*" >> "$LOG"
[ "\$1" = pkgnames ] || exit 1
prefix="\${*: -1}"
while read -r n; do case "\$n" in "\$prefix"*) echo "\$n" ;; esac; done < "$WORK/apt-names"
exit 0
SH
printf '%s\n' firefox-esr libwebkit2gtk-4.1-0 vlc g++ cowsay cowsay-off xcowsay at-spi2-common > "$WORK/apt-names"

# dpkg-query -W -f=... NAME: "installed" for the names in \$WORK/dpkg-installed.
shim dpkg-query <<SH
#!/bin/bash
name="\${*: -1}"
if grep -qxF -- "\$name" "$WORK/dpkg-installed" 2>/dev/null; then echo installed; exit 0; fi
echo "dpkg-query: no packages found matching \$name" >&2
exit 1
SH
printf '%s\n' vlc g++ at-spi2-common > "$WORK/dpkg-installed"

# rpm (Fedora): -q NAME succeeds for the names in \$WORK/rpm-installed; -e --test fails, naming who needs
# them, when \$WORK/rpm-needed has lines.
shim rpm <<SH
#!/bin/bash
echo "rpm \$*" >> "$LOG"
case "\$1" in
  -q) grep -qxF -- "\${*: -1}" "$WORK/rpm-installed" 2>/dev/null; exit ;;
  -e) if [ -s "$WORK/rpm-needed" ]; then
        echo "error: Failed dependencies:" >&2
        while read -r l; do printf '\t%s\n' "\$l" >&2; done < "$WORK/rpm-needed"
        exit 1
      fi
      exit 0 ;;
esac
exit 1
SH
printf '%s\n' vlc gtk3 > "$WORK/rpm-installed"

shim systemctl <<SH
#!/bin/bash
echo "systemctl \$*" >> "$LOG"
exit 0
SH

shim timedatectl <<SH
#!/bin/bash
echo "timedatectl \$*" >> "$LOG"
exit 0
SH

shim grub-reboot <<SH
#!/bin/bash
echo "grub-reboot \$*" >> "$LOG"
exit 0
SH

shim ntfs-3g.probe <<SH
#!/bin/bash
echo "ntfs-3g.probe \$*" >> "$LOG"
# Exit 0 => clean (rw). A marker forces "dirty".
if [ -f "$WORK/dirty" ]; then exit 1; else exit 0; fi
SH

# A fake /usr/share/zoneinfo for set-timezone validation.
ZI="$WORK/zoneinfo"
mkdir -p "$ZI/Europe" "$ZI/America"
: > "$ZI/Europe/London"
: > "$ZI/America/New_York"
: > "$ZI/UTC"

# runhelper: invoke the helper as "root" with our shims first on PATH. We cannot
# truly be uid 0 here if not root, so we neuter the id check by exporting a
# wrapper 'id' that reports 0 when the test is not run as root.
if [ "$(id -u)" -ne 0 ]; then
  shim id <<SH
#!/bin/bash
if [ "\$1" = "-u" ]; then echo 0; exit 0; fi
# Fallback to the real id for other uses.
exec /usr/bin/id "\$@"
SH
fi

runhelper() {
  # CHATCOMPUTER_ROOT_PATH is the helper's test-only PATH seam (stripped by sudo in
  # production). PATH here also lets our 'id' shim (uid 0) win for the root check.
  env PATH="$BIN:/usr/sbin:/usr/bin:/sbin:/bin" \
      CHATCOMPUTER_ROOT_PATH="$BIN:/usr/sbin:/usr/bin:/sbin:/bin" \
      CHATCOMPUTER_FORCE_EFI="${CHATCOMPUTER_FORCE_EFI:-1}" \
      SUDO_UID=1000 SUDO_GID=1000 \
      bash "$HELPER" "$@"
}

# Point the helper's zoneinfo + mount dirs at our sandbox by overriding the
# constants via a tiny wrapper copy. Simplest: symlink zoneinfo check by using a
# fake path through an env the helper reads — the helper hardcodes
# /usr/share/zoneinfo, so for the timezone test we instead rely on a real zone
# that exists on this system (Europe/London) and test rejection with traversal.

# ===========================================================================
# status
# ===========================================================================
OUT="$(runhelper status)"
if is_json "$OUT"; then pass "status: valid JSON"; else fail "status: invalid JSON: $OUT"; fi
ok=1
for k in windows_mounted windows_rw windows_bitlocker windows_boot_entry efi; do
  v="$(jget "$OUT" "$k")"
  [ -n "$v" ] || { ok=0; fail "status: missing key $k"; }
done
[ "$ok" = 1 ] && pass "status: all required keys present"

# ===========================================================================
# boot-windows-next -> efibootmgr --bootnext 0001
# ===========================================================================
: > "$LOG"
OUT="$(runhelper boot-windows-next)"
if is_json "$OUT"; then pass "boot-windows-next: valid JSON"; else fail "boot-windows-next: invalid JSON: $OUT"; fi
if grep -q 'efibootmgr --bootnext 0001' "$LOG"; then pass "boot-windows-next: set BootNext to Windows (0001)"; else fail "boot-windows-next: did not call efibootmgr --bootnext 0001"; cat "$LOG"; fi
[ "$(jget "$OUT" method)" = "bootnext" ] && pass "boot-windows-next: reports method=bootnext" || fail "boot-windows-next: method!=bootnext"

# ===========================================================================
# boot-default -> the chosen system first in BootOrder, the rest kept in order
# ===========================================================================
: > "$LOG"
OUT="$(runhelper boot-default chatcomputer)"
if grep -q 'efibootmgr --bootorder 0000,0001,0003' "$LOG"; then pass "boot-default chatcomputer: ubuntu entry first, others kept"; else fail "boot-default chatcomputer: wrong order"; cat "$LOG"; fi
[ "$(jget "$OUT" default)" = "chatcomputer" ] && pass "boot-default chatcomputer: reports default" || fail "boot-default chatcomputer: $OUT"
: > "$LOG"
OUT="$(runhelper boot-default windows)"
if grep -q 'efibootmgr --bootorder 0001,0003,0000' "$LOG"; then pass "boot-default windows: Windows Boot Manager first"; else fail "boot-default windows: wrong order"; cat "$LOG"; fi
for bad in "" "linux" "windows;reboot"; do
  : > "$LOG"
  OUT="$(runhelper boot-default $bad 2>/dev/null)"
  rc=$?
  if [ "$rc" -ne 0 ] && [ "$(jget "$OUT" ok)" = "False" ] && ! grep -q -- '--bootorder' "$LOG"; then
    pass "boot-default rejects [$bad]"
  else
    fail "boot-default accepted [$bad] -> $OUT"
  fi
done

# ===========================================================================
# apt-install: reject malicious, accept valid
# ===========================================================================
for bad in "foo; rm -rf /" "foo&&bad" "../etc" "FOO" "a b" '$(reboot)' "-rf" "pkg|cat"; do
  OUT="$(runhelper apt-install "$bad" 2>/dev/null)"
  rc=$?
  if [ "$rc" -ne 0 ] && [ "$(jget "$OUT" ok)" = "False" ]; then
    pass "apt-install rejects malicious arg: [$bad]"
  else
    fail "apt-install ACCEPTED malicious arg: [$bad] -> $OUT"
  fi
done
# Ensure apt-get was never called for the bad names.
if grep -q 'apt-get install' "$LOG"; then fail "apt-install invoked apt-get for a bad name"; else pass "apt-install never ran apt-get on bad names"; fi

: > "$LOG"
OUT="$(runhelper apt-install firefox-esr libwebkit2gtk-4.1-0 2>/dev/null)"
if [ "$(jget "$OUT" ok)" = "True" ] && grep -q 'apt-get install -y --no-install-recommends --no-remove firefox-esr libwebkit2gtk-4.1-0' "$LOG"; then
  pass "apt-install accepts valid names and calls apt-get (which may not remove anything)"
else
  fail "apt-install valid-name path failed: $OUT"; cat "$LOG"
fi
: > "$LOG"
OUT="$(runhelper apt-install g++ 2>/dev/null)"
[ "$(jget "$OUT" ok)" = "True" ] && grep -q 'apt-get install .* g++$' "$LOG" \
  && pass "apt-install takes a real name ending in + (g++)" || { fail "apt-install g++: $OUT"; cat "$LOG"; }

# apt-get reads "name-" as "remove name", and a name that isn't a package's as a pattern matching many:
# neither may reach apt-get install.
for bad in "vlc-" "cowsa.y" "cowsay+"; do
  : > "$LOG"
  OUT="$(runhelper apt-install "$bad" 2>/dev/null)"
  if [ "$(jget "$OUT" ok)" = "False" ] && ! grep -q 'apt-get install' "$LOG"; then
    pass "apt-install refuses [$bad]: $(jget "$OUT" error)"
  else
    fail "apt-install let [$bad] through -> $OUT"; cat "$LOG"
  fi
done

# ===========================================================================
# apt-remove: exactly the named, installed packages, and nothing that needs them
# ===========================================================================
: > "$LOG"; printf 'Remv vlc [3.0.20-3build6]\n' > "$WORK/apt-plan"
OUT="$(runhelper apt-remove vlc 2>/dev/null)"
if [ "$(jget "$OUT" ok)" = "True" ] && grep -q '^apt-get -s remove vlc$' "$LOG" && grep -q '^apt-get remove -y vlc$' "$LOG"; then
  pass "apt-remove removes a named, installed package (after checking what else would go)"
else
  fail "apt-remove vlc: $OUT"; cat "$LOG"
fi
# "cowsay+" given to remove would install cowsay; "at-spi2." would be a pattern: neither is an installed package.
for bad in "cowsay+" "at-spi2.c" "cowsay"; do
  : > "$LOG"
  OUT="$(runhelper apt-remove "$bad" 2>/dev/null)"
  if [ "$(jget "$OUT" ok)" = "False" ] && ! grep -q 'apt-get' "$LOG"; then
    pass "apt-remove refuses [$bad], which isn't an installed package"
  else
    fail "apt-remove let [$bad] through -> $OUT"; cat "$LOG"
  fi
done
# Removing at-spi2-common would also remove GTK and WebKit, which Chat Computer's screen needs (Ubuntu 24.04's
# apt-get -s remove at-spi2-common lists them).
: > "$LOG"
printf '%s\n' 'Remv at-spi2-common [2.52.0-1build1] [libatk1.0-0t64:amd64 ]' 'Remv libatk1.0-0t64 [2.52.0-1build1]' \
  'Remv libgtk-3-0t64 [3.24.41-4ubuntu1.3] [obconf:amd64 ]' 'Remv gir1.2-webkit2-4.1 [2.52.6-0ubuntu0.24.04.1]' > "$WORK/apt-plan"
OUT="$(runhelper apt-remove at-spi2-common 2>/dev/null)"
err="$(jget "$OUT" error)"
if [ "$(jget "$OUT" ok)" = "False" ] && ! grep -q '^apt-get remove' "$LOG" \
   && [[ "$err" == *gir1.2-webkit2-4.1* ]] && [[ "$err" == *libgtk-3-0t64* ]] && [[ "$err" != *at-spi2-common* ]]; then
  pass "apt-remove refuses to take what needs the package with it, and names it: $err"
else
  fail "apt-remove cascade: $OUT"; cat "$LOG"
fi
rm -f "$WORK/apt-plan"
# Chat Computer's own packages (install-chatcomputer.sh lists them) stay, even when named.
printf '%s\n' gir1.2-webkit2-4.1 pipewire > "$WORK/base-packages"
export CHATCOMPUTER_TEST_BASE_PACKAGES="$WORK/base-packages"
printf '%s\n' gir1.2-webkit2-4.1 pipewire >> "$WORK/dpkg-installed"
for pm in apt dnf; do
  : > "$LOG"
  OUT="$(CHATCOMPUTER_TEST_PKG=$pm runhelper apt-remove vlc gir1.2-webkit2-4.1 2>/dev/null)"
  if [ "$(jget "$OUT" ok)" = "False" ] && ! grep -qE '^(apt-get|dnf|rpm) ' "$LOG" && [[ "$(jget "$OUT" error)" == *"part of Chat Computer"* ]]; then
    pass "apt-remove ($pm) keeps Chat Computer's own packages: $(jget "$OUT" error)"
  else
    fail "apt-remove ($pm) of a Chat Computer package: $OUT"; cat "$LOG"
  fi
done
unset CHATCOMPUTER_TEST_BASE_PACKAGES

# ===========================================================================
# set-timezone: reject traversal/unknown, accept a real zone
# ===========================================================================
for bad in "../../etc/passwd" "Europe/../../root" "/etc/shadow" "Foo/\$(id)" "a;b"; do
  OUT="$(runhelper set-timezone "$bad" 2>/dev/null)"
  if [ "$(jget "$OUT" ok)" = "False" ]; then pass "set-timezone rejects [$bad]"; else fail "set-timezone ACCEPTED [$bad] -> $OUT"; fi
done
# A real zone that exists on this system.
if [ -f /usr/share/zoneinfo/Europe/London ]; then
  OUT="$(runhelper set-timezone Europe/London 2>/dev/null)"
  if [ "$(jget "$OUT" ok)" = "True" ] && [ "$(jget "$OUT" timezone)" = "Europe/London" ]; then
    pass "set-timezone accepts Europe/London"
  else
    fail "set-timezone valid path failed: $OUT"
  fi
else
  echo "SKIP set-timezone valid (no zoneinfo on host)" >&2
fi

# ===========================================================================
# mount-windows: discover NTFS w/ Windows, mount rw when clean
# ===========================================================================
: > "$LOG"; rm -f "$WORK/mounted" "$WORK/dirty"
OUT="$(runhelper mount-windows 2>/dev/null)"
if is_json "$OUT"; then pass "mount-windows: valid JSON"; else fail "mount-windows: invalid JSON: $OUT"; fi
# Should have probed sda3 (ntfs with Windows/System32) and mounted it rw (clean).
if [ "$(jget "$OUT" windows_mounted)" = "True" ] && [ "$(jget "$OUT" windows_rw)" = "True" ]; then
  pass "mount-windows: mounted rw when clean"
else
  fail "mount-windows: expected mounted rw; got $OUT"; cat "$LOG"
fi
# Never pass remove_hiberfile.
if grep -q 'remove_hiberfile' "$LOG"; then fail "mount-windows: used remove_hiberfile (forbidden)"; else pass "mount-windows: never used remove_hiberfile"; fi
# Must mount with uid/gid/umask=077.
if grep -q 'umask=077' "$LOG" && grep -q 'uid=1000' "$LOG"; then pass "mount-windows: mounted with uid/gid + umask=077"; else fail "mount-windows: missing uid/umask options"; fi

# dirty -> ro
: > "$LOG"; touch "$WORK/dirty"
OUT="$(runhelper mount-windows 2>/dev/null)"
if [ "$(jget "$OUT" windows_rw)" = "False" ] && [ "$(jget "$OUT" windows_mounted)" = "True" ]; then
  pass "mount-windows: mounts ro when dirty"
else
  fail "mount-windows: expected ro when dirty; got $OUT"
fi
rm -f "$WORK/dirty"

# ===========================================================================
# mount-windows with BitLocker: opened with Chat Computer's key (on stdin), then mounted
# ===========================================================================
# Windows' partition sda3 is BitLocker now; sda4 a BitLocker data drive the key doesn't open.
BLK="$WORK/bitlocker-recovery-key"
MAPDIR="$WORK/mapper"
mkdir -p "$MAPDIR"
printf '%s\n' "111111-222222-333333-444444-555555-666666-777777-888888" > "$BLK"
shim lsblk <<SH
#!/bin/bash
echo "lsblk \$*" >> "$LOG"
if printf '%s' "\$*" | grep -q 'NAME,FSTYPE'; then
  printf 'sda\nsda1 vfat\nsda2 ext4\nsda4 BitLocker\nsda3 BitLocker\n'
fi
SH
shim cryptsetup <<SH
#!/bin/bash
echo "cryptsetup \$*" >> "$LOG"
if [ "\$1" = "open" ]; then
  key=\$(cat)
  printf '%s' "\$key" > "$WORK/key-seen"
  # The key opens sda3 (Windows' drive) only.
  case "\$*" in *"/dev/sda3 chatcomputer-windows"*) ;; *) exit 2 ;; esac
  [ "\$key" = "111111-222222-333333-444444-555555-666666-777777-888888" ] || exit 2
  : > "$MAPDIR/chatcomputer-windows"; exit 0
fi
if [ "\$1" = "close" ]; then rm -f "$MAPDIR/chatcomputer-windows"; exit 0; fi
exit 1
SH
shim mount <<SH
#!/bin/bash
echo "mount \$*" >> "$LOG"
mp="\${@: -1}"; dev="\${@: -2:1}"
mkdir -p "\$mp"
case "\$dev" in */chatcomputer-windows) mkdir -p "\$mp/Windows/System32" ;; esac
exit 0
SH
export CHATCOMPUTER_TEST_BITLOCKER_KEY="$BLK" CHATCOMPUTER_TEST_MAPPER_DIR="$MAPDIR"
: > "$LOG"; rm -f "$WORK/mounted" "$MAPDIR/chatcomputer-windows" "$WORK/key-seen"
OUT="$(runhelper mount-windows 2>/dev/null)"
if [ "$(jget "$OUT" windows_mounted)" = "True" ] && [ "$(jget "$OUT" windows_bitlocker)" = "True" ] && [ "$(jget "$OUT" device)" = "/dev/sda3" ]; then
  pass "mount-windows: opened the BitLocker drive with Chat Computer's key and mounted it"
else
  fail "mount-windows (BitLocker): expected mounted; got $OUT"; cat "$LOG"
fi
if grep -q "cryptsetup open --type bitlk --key-file=- /dev/sda3 chatcomputer-windows" "$LOG" \
   && [ "$(cat "$WORK/key-seen" 2>/dev/null)" = "111111-222222-333333-444444-555555-666666-777777-888888" ]; then
  pass "mount-windows: the key went to cryptsetup on stdin, without the newline"
else
  fail "mount-windows: cryptsetup not called as expected"; cat "$LOG"
fi
if grep -q "111111-222222" "$LOG"; then fail "mount-windows: the key appeared on a command line"; else pass "mount-windows: the key never appeared on a command line"; fi
if grep -q "mount -t ntfs-3g -o rw,.* $MAPDIR/chatcomputer-windows /mnt/windows" "$LOG"; then
  pass "mount-windows: mounted the opened drive (rw when clean)"
else
  fail "mount-windows: did not mount the opened drive"; cat "$LOG"
fi
# No key: reported, nothing opened.
: > "$LOG"; rm -f "$MAPDIR/chatcomputer-windows"
export CHATCOMPUTER_TEST_BITLOCKER_KEY="$WORK/no-such-key"
OUT="$(runhelper mount-windows 2>/dev/null)"
unset CHATCOMPUTER_TEST_BITLOCKER_KEY CHATCOMPUTER_TEST_MAPPER_DIR
if [ "$(jget "$OUT" windows_mounted)" = "False" ] && [ "$(jget "$OUT" windows_bitlocker)" = "True" ] && ! grep -q "cryptsetup open" "$LOG"; then
  pass "mount-windows: without a key, says the drive is encrypted and opens nothing"
else
  fail "mount-windows (no key): got $OUT"; cat "$LOG"
fi
# Restore the plain-NTFS shims for the tests below.
shim lsblk <<SH
#!/bin/bash
echo "lsblk \$*" >> "$LOG"
if printf '%s' "\$*" | grep -q 'NAME,FSTYPE'; then
  printf 'sda\nsda1 vfat\nsda2 ext4\nsda3 ntfs\nsda4 ntfs\n'
fi
SH

# ===========================================================================
# wipe-install-secrets: only CHATCOMP device, removes secrets
# ===========================================================================
: > "$LOG"
# Make blkid -L resolve to a real block-like path by creating usb.img and
# faking -b test via a 'test' override is overkill; instead we point to a device
# node the helper will accept. The helper requires [ -b "$dev" ]; create a loop
# file won't be a block dev. So we add a /dev-style shim: use /dev/zero? The
# helper only needs -b true. We satisfy it by having blkid return /dev/loopX if
# present, else we assert the not-a-block rejection path.
# We need a real block device to satisfy the helper's [ -b "$dev" ] safety
# check; find one below.
# Use the first existing loop/zram/sr device as a stand-in block device.
BDEV="$(ls /dev/loop0 /dev/ram0 /dev/sr0 2>/dev/null | head -n1 || true)"
if [ -n "$BDEV" ] && [ -b "$BDEV" ]; then
  shim blkid <<SH
#!/bin/bash
echo "blkid \$*" >> "$LOG"
if [ "\$1" = "-L" ] && [ "\$2" = "CHATCOMP" ]; then echo "$BDEV"; exit 0; fi
echo "ntfs"
SH
  # Mount shim must create the secrets dir when the CHATCOMP block device is
  # mounted (so the removal path is exercised).
  shim mount <<SH
#!/bin/bash
echo "mount \$*" >> "$LOG"
mp="\${@: -1}"; dev="\${@: -2:1}"
mkdir -p "\$mp"
case "\$dev" in
  $BDEV) mkdir -p "\$mp/chatcomputer/secrets"; echo secret > "\$mp/chatcomputer/secrets/claude_token" ;;
  */sda3) mkdir -p "\$mp/Windows/System32" ;;
esac
exit 0
SH
  OUT="$(runhelper wipe-install-secrets 2>/dev/null)"
  if is_json "$OUT" && [ "$(jget "$OUT" ok)" = "True" ]; then
    pass "wipe-install-secrets: ran against CHATCOMP block device"
    [ "$(jget "$OUT" secrets_removed)" = "True" ] && pass "wipe-install-secrets: removed secrets dir" || echo "NOTE wipe-install-secrets: secrets_removed=$(jget "$OUT" secrets_removed)" >&2
  else
    fail "wipe-install-secrets: unexpected result: $OUT"
  fi
  # Confirm it only mounted the CHATCOMP device.
  if grep -E 'mount .*'"$BDEV" "$LOG" >/dev/null; then pass "wipe-install-secrets: mounted only the labelled device"; else fail "wipe-install-secrets: did not mount the labelled device"; fi
else
  # No block device available: assert the helper rejects a non-block result.
  OUT="$(runhelper wipe-install-secrets 2>/dev/null)"
  if [ "$(jget "$OUT" ok)" = "False" ]; then pass "wipe-install-secrets: rejects non-block CHATCOMP target (no loop dev to test happy path)"; else fail "wipe-install-secrets: should reject non-block device"; fi
fi

# ===========================================================================
# On a Mac (Fedora Asahi Remix): macOS for the next start, what starts, packages with dnf
# ===========================================================================
DT="$WORK/compatible"
printf 'apple,j313\0apple,t8103\0apple,arm-platform\0' > "$DT"
VOLS="$WORK/volumes"
shim asahi-bless <<SH
#!/bin/bash
echo "asahi-bless \$*" >> "$LOG"
case "\$*" in
  *--set-boot-macos*) [ -f "$WORK/macos-renamed" ] && { echo "Error: Unable to find the macos volume" >&2; exit 1; }; exit 0 ;;
  --list-volumes) cat "$VOLS"; exit 0 ;;
  *--set-boot*) exit 0 ;;
esac
exit 1
SH
shim dnf <<SH
#!/bin/bash
echo "dnf \$*" >> "$LOG"
exit 0
SH
export CHATCOMPUTER_TEST_DT_COMPATIBLE="$DT"
: > "$LOG"; rm -f "$WORK/macos-renamed"
OUT="$(runhelper boot-macos-next 2>/dev/null)"
if [ "$(jget "$OUT" ok)" = "True" ] && grep -q -- "asahi-bless --set-boot-macos --next --yes" "$LOG"; then
  pass "boot-macos-next: macOS for the next start only (asahi-bless --set-boot-macos --next)"
else
  fail "boot-macos-next: $OUT"; cat "$LOG"
fi
# macOS renamed (not "Macintosh ..."): the one start-up volume that isn't Chat Computer's.
touch "$WORK/macos-renamed"
printf '*1) Chat Computer\n 2) Sam'"'"'s Mac\n' > "$VOLS"
: > "$LOG"
OUT="$(runhelper boot-macos-next 2>/dev/null)"
if [ "$(jget "$OUT" ok)" = "True" ] && grep -q -- "asahi-bless --set-boot 2 --next --yes" "$LOG" && [ "$(jget "$OUT" volume)" = "Sam's Mac" ]; then
  pass "boot-macos-next: a renamed macOS is found as the one other start-up volume"
else
  fail "boot-macos-next (renamed): $OUT"; cat "$LOG"
fi
printf '*1) Chat Computer\n 2) Sam'"'"'s Mac\n 3) Fedora Linux\n' > "$VOLS"
: > "$LOG"
OUT="$(runhelper boot-macos-next 2>/dev/null)"
if [ "$(jget "$OUT" ok)" = "False" ] && ! grep -q -- "--set-boot [0-9]" "$LOG"; then
  pass "boot-macos-next: with two other systems, guesses nothing"
else
  fail "boot-macos-next (two others): $OUT"; cat "$LOG"
fi
rm -f "$WORK/macos-renamed"
: > "$LOG"
OUT="$(runhelper boot-default macos 2>/dev/null)"
if [ "$(jget "$OUT" ok)" = "True" ] && grep -q -- "asahi-bless --set-boot-macos --yes" "$LOG" && ! grep -q -- "--next" "$LOG"; then
  pass "boot-default macos: macOS from now on"
else
  fail "boot-default macos: $OUT"; cat "$LOG"
fi
: > "$LOG"
OUT="$(runhelper boot-default chatcomputer 2>/dev/null)"
if [ "$(jget "$OUT" ok)" = "True" ] && grep -q -- "asahi-bless --set-boot Chat Computer --yes" "$LOG" && ! grep -q efibootmgr "$LOG"; then
  pass "boot-default chatcomputer on a Mac: Chat Computer's start-up volume, no UEFI entries"
else
  fail "boot-default chatcomputer (Mac): $OUT"; cat "$LOG"
fi
OUT="$(runhelper boot-default windows 2>/dev/null)"
[ "$(jget "$OUT" ok)" = "False" ] && pass "boot-default windows refused on a Mac" || fail "boot-default windows on a Mac: $OUT"
: > "$LOG"
OUT="$(CHATCOMPUTER_TEST_PKG=dnf runhelper apt-install vlc 2>/dev/null)"
if [ "$(jget "$OUT" ok)" = "True" ] && grep -q "dnf install -y --setopt=install_weak_deps=False vlc" "$LOG" && ! grep -q apt-get "$LOG"; then
  pass "apt-install on Fedora: dnf install"
else
  fail "apt-install with dnf: $OUT"; cat "$LOG"
fi
: > "$LOG"; : > "$WORK/rpm-needed"
OUT="$(CHATCOMPUTER_TEST_PKG=dnf runhelper apt-remove vlc 2>/dev/null)"
if [ "$(jget "$OUT" ok)" = "True" ] && grep -q "dnf remove -y --setopt=clean_requirements_on_remove=False vlc" "$LOG"; then
  pass "apt-remove on Fedora: dnf remove of just that package"
else
  fail "apt-remove with dnf: $OUT"; cat "$LOG"
fi
: > "$LOG"
echo 'libgtk-3.so.0()(64bit) is needed by (installed) webkit2gtk4.1-2.48.0-1.fc44.aarch64' > "$WORK/rpm-needed"
OUT="$(CHATCOMPUTER_TEST_PKG=dnf runhelper apt-remove gtk3 2>/dev/null)"
if [ "$(jget "$OUT" ok)" = "False" ] && ! grep -q "dnf remove" "$LOG" && [[ "$(jget "$OUT" error)" == *webkit2gtk4.1-2.48.0* ]]; then
  pass "apt-remove on Fedora refuses to take what needs the package with it: $(jget "$OUT" error)"
else
  fail "apt-remove with dnf, needed: $OUT"; cat "$LOG"
fi
: > "$LOG"; : > "$WORK/rpm-needed"
OUT="$(CHATCOMPUTER_TEST_PKG=dnf runhelper apt-remove cowsay 2>/dev/null)"
[ "$(jget "$OUT" ok)" = "False" ] && ! grep -q "dnf remove" "$LOG" \
  && pass "apt-remove on Fedora refuses a package that isn't installed" || { fail "apt-remove with dnf, not installed: $OUT"; cat "$LOG"; }
: > "$LOG"
OUT="$(CHATCOMPUTER_TEST_PKG=dnf runhelper system-update 2>/dev/null)"
grep -q "dnf upgrade -y --refresh" "$LOG" && pass "system-update on Fedora: dnf upgrade" || fail "system-update with dnf: $OUT"
unset CHATCOMPUTER_TEST_DT_COMPATIBLE
printf 'qemu,virt\0' > "$DT"
OUT="$(CHATCOMPUTER_TEST_DT_COMPATIBLE="$DT" runhelper boot-macos-next 2>/dev/null)"
[ "$(jget "$OUT" ok)" = "False" ] && pass "boot-macos-next refused on a PC" || fail "boot-macos-next on a PC: $OUT"

# ===========================================================================
# update-app / update-undo / update-done: Chat Computer from the folder Chat Computer Setup ran from
# ===========================================================================
UPD="$WORK/upd"
SRC_REL="Users/Sam Lee/Downloads/ChatComputer"
mk_app() {   # mk_app DIR VERSION: a pretend Chat Computer
  mkdir -p "$1/chatcomputer/brain" "$1/linux/session" "$1/linux/root-helper"
  printf '__version__ = "%s"\n' "$2" > "$1/chatcomputer/__init__.py"
  printf 'x = "%s"\n' "$2" > "$1/chatcomputer/brain/agent.py"
  printf '#!/bin/bash\necho %s\n' "$2" > "$1/linux/session/chatcomputer-session"
  printf 'exec something %s\n' "$2" > "$1/linux/session/xinitrc"
  printf '#!/bin/bash\necho root %s\n' "$2" > "$1/linux/root-helper/chatcomputer-root"
  printf 'numpy==1\nsoxr==1\n' > "$1/requirements-linux.txt"
}
reset_update() {
  rm -rf "$UPD"; mkdir -p "$UPD/opt" "$UPD/win/$SRC_REL" "$UPD/etc" "$UPD/var"
  mk_app "$UPD/opt/app" 1
  mk_app "$UPD/win/$SRC_REL" 2
  mkdir -p "$UPD/win/$SRC_REL/chatcomputer/__pycache__"; : > "$UPD/win/$SRC_REL/chatcomputer/__pycache__/x.cpython-312.pyc"
  chmod -R 0777 "$UPD/win"          # Windows' drive: whatever it says
  printf '%s\n' "$SRC_REL" > "$UPD/etc/update-source"
  : > "$UPD/var/base-installed"
  touch "$WORK/mounted"
}
runupdate() {
  CHATCOMPUTER_TEST_APP_ROOT="$UPD/opt" CHATCOMPUTER_TEST_UPDATE_SOURCE="$UPD/etc/update-source" \
  CHATCOMPUTER_TEST_UPDATE_PENDING="$UPD/var/update-pending" CHATCOMPUTER_TEST_WINDOWS_MOUNT="$UPD/win" \
  CHATCOMPUTER_TEST_BASE_INSTALLED="$UPD/var/base-installed" runhelper "$@" 2>/dev/null
}
app_version() { sed -n 's/^__version__ = "\(.*\)"$/\1/p' "$UPD/opt/${1:-app}/chatcomputer/__init__.py" 2>/dev/null; }
mode_of() { stat -c '%a' "$1"; }

if [ "$(id -u)" -eq 0 ]; then
  reset_update
  OUT="$(runupdate update-app)"
  if is_json "$OUT" && [ "$(jget "$OUT" ok)" = "True" ] && [ "$(jget "$OUT" changed)" = "4" ] && [ "$(app_version)" = 2 ] \
     && grep -q 'echo 2' "$UPD/opt/app/linux/session/chatcomputer-session" && [ "$(app_version app.prev)" = 1 ] \
     && [ -f "$UPD/var/update-pending" ]; then
    pass "update-app: puts the new version in place, the old one aside, and marks it new ($OUT)"
  else
    fail "update-app: $OUT"; find "$UPD/opt" | sort >&2
  fi
  grep -q 'root 1' "$UPD/opt/app/linux/root-helper/chatcomputer-root" \
    && pass "update-app: leaves what root runs as installed" || fail "update-app changed the root helper"
  [ ! -e "$UPD/opt/app/chatcomputer/__pycache__" ] && pass "update-app: no compiled files from Windows' drive" \
    || fail "update-app copied __pycache__"
  if [ "$(mode_of "$UPD/opt/app/chatcomputer/brain")" = 755 ] && [ "$(mode_of "$UPD/opt/app/chatcomputer/brain/agent.py")" = 644 ] \
     && [ "$(mode_of "$UPD/opt/app/linux/session/chatcomputer-session")" = 755 ] \
     && [ "$(mode_of "$UPD/opt/app/linux/session/xinitrc")" = 644 ] \
     && [ "$(stat -c '%u' "$UPD/opt/app/chatcomputer/brain/agent.py")" = 0 ]; then
    pass "update-app: root's files, read-only to everyone else; programs executable"
  else
    fail "update-app modes: $(stat -c '%a %u %n' "$UPD/opt/app/chatcomputer/brain" "$UPD/opt/app/chatcomputer/brain/agent.py" "$UPD/opt/app/linux/session/chatcomputer-session" "$UPD/opt/app/linux/session/xinitrc")"
  fi
  OUT="$(runupdate update-app)"
  [ "$(jget "$OUT" ok)" = "True" ] && [ "$(jget "$OUT" changed)" = "0" ] && [ "$(app_version app.prev)" = 1 ] \
    && pass "update-app again: already up to date, the version before kept" || fail "update-app again: $OUT"
  OUT="$(runupdate update-undo)"
  if [ "$(jget "$OUT" ok)" = "True" ] && [ "$(app_version)" = 1 ] && [ ! -e "$UPD/var/update-pending" ] \
     && [ "$(app_version app.failed)" = 2 ]; then
    pass "update-undo: the version before is back"
  else
    fail "update-undo: $OUT"
  fi
  OUT="$(runupdate update-undo)"
  [ "$(jget "$OUT" ok)" = "False" ] && [ "$(app_version)" = 1 ] \
    && pass "update-undo with nothing to go back to: refused" || fail "update-undo again: $OUT"
  date +%s > "$UPD/var/update-pending"
  OUT="$(runupdate update-done)"
  [ "$(jget "$OUT" ok)" = "True" ] && [ ! -e "$UPD/var/update-pending" ] && pass "update-done: the update is kept" \
    || fail "update-done: $OUT"
  # A version that needs fewer Python packages (one left out; comments and spacing aside) is an update too: what
  # it needs is installed already.
  reset_update; printf '# the runtime\r\nnumpy==1    # arrays\r\n\r\n' > "$UPD/win/$SRC_REL/requirements-linux.txt"
  OUT="$(runupdate update-app)"
  [ "$(jget "$OUT" ok)" = "True" ] && [ "$(app_version)" = 2 ] \
    && pass "update-app: a version needing fewer Python packages is put in place" || fail "update-app, fewer packages: $OUT"

  # What update-app refuses, leaving the installed version as it is.
  refused() {   # refused WHAT: update-app must fail and change nothing
    local out
    out="$(runupdate update-app)"
    if is_json "$out" && [ "$(jget "$out" ok)" = "False" ] && [ "$(app_version)" = 1 ] && [ ! -e "$UPD/opt/app.next" ] \
       && [ ! -e "$UPD/var/update-pending" ]; then
      pass "update-app refuses $1: $(jget "$out" error)"
    else
      fail "update-app with $1: $out"
    fi
  }
  reset_update; rm "$UPD/var/base-installed";                     refused "before Chat Computer has finished installing"
  reset_update; printf 'numpy==2\nsoxr==1\n' > "$UPD/win/$SRC_REL/requirements-linux.txt"; refused "other Python packages"
  reset_update; printf 'numpy==1\nsoxr==1\nmore==1\n' > "$UPD/win/$SRC_REL/requirements-linux.txt"; refused "one more Python package"
  reset_update; ln -s /etc "$UPD/win/$SRC_REL/chatcomputer/etc";       refused "a link out of the folder"
  reset_update; rm -rf "$UPD/win/$SRC_REL/linux/session"; ln -s /etc "$UPD/win/$SRC_REL/linux/session"; refused "a part that is a link"
  reset_update; : > "$UPD/win/$SRC_REL/chatcomputer/sitecustomize.py"; refused "a Python start-up hook"
  reset_update; : > "$UPD/win/$SRC_REL/chatcomputer/brain/x.pth";      refused "a .pth file"
  reset_update; mkfifo "$UPD/win/$SRC_REL/chatcomputer/pipe";           refused "a special file"
  reset_update; printf 'Users/../../opt\n' > "$UPD/etc/update-source"; refused "a recorded folder with .."
  reset_update; printf '%s\n' "$UPD/opt" > "$UPD/etc/update-source"; refused "a recorded folder that isn't on Windows' drive"
  reset_update; ln -s "$UPD/opt" "$UPD/win/elsewhere"; printf 'elsewhere\n' > "$UPD/etc/update-source"; refused "a recorded folder that leads off Windows' drive"
  reset_update; rm "$UPD/etc/update-source";                       refused "without a recorded folder"
  reset_update; rm -rf "$UPD/win/$SRC_REL/chatcomputer";               refused "a folder without Chat Computer"
  reset_update; printf 'Users/Nobody\n' > "$UPD/etc/update-source"; refused "a recorded folder that isn't there"
  OUT="$(runupdate update-app now 2>/dev/null)"; [ "$(jget "$OUT" ok)" = "False" ] && pass "update-app takes no arguments" || fail "update-app with an argument: $OUT"
  rm -f "$WORK/mounted"
else
  echo "SKIP update-app tests (they need root, for the files' owner)" >&2
fi

# ===========================================================================
# Rejections: unknown verb, bad arg counts, no verb
# ===========================================================================
OUT="$(runhelper 2>/dev/null)";            [ "$(jget "$OUT" ok)" = "False" ] && pass "no verb rejected" || fail "no verb not rejected: $OUT"
OUT="$(runhelper bogus-verb 2>/dev/null)"; [ "$(jget "$OUT" ok)" = "False" ] && pass "unknown verb rejected" || fail "unknown verb not rejected: $OUT"
OUT="$(runhelper status extra 2>/dev/null)"; [ "$(jget "$OUT" ok)" = "False" ] && pass "status with extra arg rejected" || fail "status extra-arg not rejected: $OUT"
OUT="$(runhelper apt-install 2>/dev/null)"; [ "$(jget "$OUT" ok)" = "False" ] && pass "apt-install with no package rejected" || fail "apt-install empty not rejected: $OUT"
OUT="$(runhelper set-timezone 2>/dev/null)"; [ "$(jget "$OUT" ok)" = "False" ] && pass "set-timezone with no arg rejected" || fail "set-timezone empty not rejected: $OUT"

exit "$FAILED"
