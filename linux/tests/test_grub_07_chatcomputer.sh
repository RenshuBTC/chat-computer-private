#!/bin/bash
# Test grub/07_chatcomputer output.
#
# Strategy: run 07_chatcomputer with a MOCKED grub-mkconfig_lib + grub-probe so it
# runs without a real GRUB install, with CHATCOMPUTER_SYSROOT pointing at a fake
# system that has the kernel links, in these setups:
#   A) Ubuntu 24.04: links in /boot, /boot on the root fs -> kernel path /boot/vmlinuz
#   B) /boot on a separate partition -> kernel path relative to that partition
#   C) older setups: links only at the filesystem root -> kernel path /vmlinuz
#   D) no kernel links at all -> no Chat Computer entry and no Windows entry, so
#      GRUB's fallback (the first entry) is Ubuntu, never Windows
# and assert:
#   * a 'menuentry 'Chat Computer'' with id 'chatcomputer'
#   * a 'menuentry 'Windows'' that chainloads bootmgfw.efi via search --file
#   * the Chat Computer entry carries root=UUID=... and the correct kernel path
#   * the generated snippet passes `grub-script-check` when available.
#
# If the real grub-common lib is installed we also run against it for extra
# fidelity.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/.." && pwd)"
SCRIPT="$REPO/grub/07_chatcomputer"
FAILED=0
pass() { printf 'PASS %s\n' "$1"; }
fail() { printf 'FAIL %s\n' "$1"; FAILED=1; }

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# --- Build a mock grub environment -----------------------------------------
# Mock grub-mkconfig_lib providing just the functions 07_chatcomputer uses.
MOCKLIB="$WORK/grub-mkconfig_lib"
cat > "$MOCKLIB" <<'LIB'
# Mock grub-mkconfig_lib for tests.
grub_tab="	"
grub_add_tab () { sed -e "s/^/$grub_tab/"; }
make_system_path_relative_to_its_root () {
  # Like grub-mkrelpath: /boot's path inside the file system that holds it, so
  # "/boot" when /boot is on the root fs and "/" when it is its own partition.
  if [ "$1" = "/boot" ]; then
    printf '%s' "${MOCK_BOOT_REL:-/boot}"
  else
    printf '%s' "$1"
  fi
}
prepare_grub_to_access_device () {
  # Emit deterministic lines like the real function would.
  echo "# access $1"
  echo "insmod part_gpt"
  echo "insmod ext2"
  echo "search --no-floppy --fs-uuid --set=root ${MOCK_BOOT_UUID:-1111-2222}"
}
gettext_printf () { :; }   # silence info messages
LIB

# Mock grub-probe: answer the --target queries 07_chatcomputer may use standalone.
mkdir -p "$WORK/bin"
cat > "$WORK/bin/grub-probe" <<'PROBE'
#!/bin/sh
# Parse the --target and echo a fixed answer.
target=""
for a in "$@"; do case "$a" in --target=*) target=${a#--target=};; esac; done
case "$target" in
  device) echo "/dev/sda2" ;;
  *) echo "" ;;
esac
PROBE
chmod +x "$WORK/bin/grub-probe"

# grub-script-check may exist on the system; if so we use the real one.

run_script() {
  # $1 = scenario label, remaining env vars already exported by caller.
  env \
    PATH="$WORK/bin:$PATH" \
    pkgdatadir="$WORK" \
    grub_probe="$WORK/bin/grub-probe" \
    GRUB_DEVICE="/dev/sda2" \
    GRUB_DEVICE_UUID="DEAD-BEEF" \
    GRUB_DEVICE_BOOT="${GRUB_DEVICE_BOOT:-/dev/sda2}" \
    GRUB_CMDLINE_LINUX="" \
    GRUB_CMDLINE_LINUX_DEFAULT="quiet splash" \
    MOCK_BOOT_REL="${MOCK_BOOT_REL:-/boot}" \
    MOCK_BOOT_UUID="${MOCK_BOOT_UUID:-DEAD-BEEF}" \
    CHATCOMPUTER_SYSROOT="${SYSROOT:-$WORK/sys-boot-links}" \
    bash "$SCRIPT" 2>/dev/null
}

# Fake systems: kernel links in /boot (Ubuntu 24.04), at the root (older), or none.
mkdir -p "$WORK/sys-boot-links/boot" "$WORK/sys-root-links" "$WORK/sys-none/boot"
ln -s vmlinuz-7.0.0-38-generic "$WORK/sys-boot-links/boot/vmlinuz"
ln -s initrd.img-7.0.0-38-generic "$WORK/sys-boot-links/boot/initrd.img"
: > "$WORK/sys-boot-links/boot/vmlinuz-7.0.0-38-generic"
: > "$WORK/sys-boot-links/boot/initrd.img-7.0.0-38-generic"
mkdir -p "$WORK/sys-root-links/boot"
: > "$WORK/sys-root-links/boot/vmlinuz-6.8.0-1-generic"
: > "$WORK/sys-root-links/boot/initrd.img-6.8.0-1-generic"
ln -s boot/vmlinuz-6.8.0-1-generic "$WORK/sys-root-links/vmlinuz"
ln -s boot/initrd.img-6.8.0-1-generic "$WORK/sys-root-links/initrd.img"

# Create the by-uuid symlink so the script's UUID branch (matching 10_linux's
# "use UUID only if /dev/disk/by-uuid/<uuid> exists") is exercised. If we can't
# (no permission), the script correctly falls back to the device node and the
# UUID assertions are softened below.
UUIDLINK_CREATED=0
if [ ! -e /dev/disk/by-uuid/DEAD-BEEF ]; then
  mkdir -p /dev/disk/by-uuid 2>/dev/null \
    && ln -s /dev/null /dev/disk/by-uuid/DEAD-BEEF 2>/dev/null \
    && UUIDLINK_CREATED=1
fi
cleanup_links() {
  [ "$UUIDLINK_CREATED" = 1 ] && rm -f /dev/disk/by-uuid/DEAD-BEEF
}
trap 'cleanup_links; rm -rf "$WORK"' EXIT
EXPECT_UUID=0
[ -e /dev/disk/by-uuid/DEAD-BEEF ] && EXPECT_UUID=1

# --- Scenario A: /boot on root fs -----------------------------------------
export MOCK_BOOT_REL="/boot"
export GRUB_DEVICE_BOOT="/dev/sda2"
OUT_A="$(run_script A)"
printf '%s\n' "$OUT_A" > "$WORK/out_a.cfg"

echo "----- scenario A (/boot on root) output -----" >&2
printf '%s\n' "$OUT_A" | sed 's/^/A| /' >&2

# Windows entry assertions (always expected).
if printf '%s' "$OUT_A" | grep -q "menuentry 'Windows'"; then pass "A: Windows menuentry present"; else fail "A: Windows menuentry missing"; fi
if printf '%s' "$OUT_A" | grep -q "search --file --no-floppy --set=root /EFI/Microsoft/Boot/bootmgfw.efi"; then pass "A: Windows search --file for bootmgfw.efi"; else fail "A: Windows search --file missing"; fi
if printf '%s' "$OUT_A" | grep -q "chainloader (\$root)/EFI/Microsoft/Boot/bootmgfw.efi"; then pass "A: Windows chainloader present"; else fail "A: Windows chainloader missing"; fi
if printf '%s' "$OUT_A" | grep -q "menuentry_id_option 'chatcomputer-windows'"; then pass "A: Windows entry id"; else fail "A: Windows entry id missing"; fi

if printf '%s' "$OUT_A" | grep -q "menuentry 'Chat Computer' .* 'chatcomputer' {"; then pass "A: Chat Computer menuentry with id chatcomputer"; else fail "A: Chat Computer menuentry missing/id wrong"; fi
if [ "$EXPECT_UUID" = 1 ]; then
  if printf '%s' "$OUT_A" | grep -q "linux	/boot/vmlinuz root=UUID=DEAD-BEEF ro"; then pass "A: kernel path /boot/vmlinuz with root=UUID"; else fail "A: kernel line wrong (expected /boot/vmlinuz root=UUID=DEAD-BEEF)"; fi
else
  if printf '%s' "$OUT_A" | grep -q "linux	/boot/vmlinuz root=/dev/sda2 ro"; then pass "A: kernel path /boot/vmlinuz with device fallback (no by-uuid link here)"; else fail "A: kernel line wrong (expected /boot/vmlinuz root=/dev/sda2)"; fi
fi
if printf '%s' "$OUT_A" | grep -q "initrd	/boot/initrd.img"; then pass "A: initrd path /boot/initrd.img"; else fail "A: initrd path wrong"; fi
if printf '%s' "$OUT_A" | grep -q "search --no-floppy --fs-uuid --set=root"; then pass "A: prepare_grub_to_access_device emitted"; else fail "A: prepare_grub_to_access_device missing"; fi
if [ "$(printf '%s\n' "$OUT_A" | grep -n "menuentry 'Chat Computer'" | cut -d: -f1)" -lt "$(printf '%s\n' "$OUT_A" | grep -n "menuentry 'Windows'" | cut -d: -f1)" ]; then pass "A: Chat Computer comes before Windows"; else fail "A: Windows comes first"; fi

# --- Scenario B: /boot on a separate partition -----------------------------
# When /boot is its own partition, make_system_path_relative_to_its_root /boot
# is "/": the kernel sits at the top of that partition.
export MOCK_BOOT_REL="/"
export GRUB_DEVICE_BOOT="/dev/sda1"
OUT_B="$(run_script B)"
printf '%s\n' "$OUT_B" > "$WORK/out_b.cfg"

echo "----- scenario B (/boot separate) output -----" >&2
printf '%s\n' "$OUT_B" | sed 's/^/B| /' >&2

if printf '%s' "$OUT_B" | grep -q "linux	/vmlinuz root=UUID=DEAD-BEEF ro\|linux	/vmlinuz root=/dev/sda2 ro"; then pass "B: kernel at the top of the boot partition, root still the root fs"; else fail "B: kernel path wrong (expected /vmlinuz)"; fi
if printf '%s' "$OUT_B" | grep -q "initrd	/initrd.img"; then pass "B: initrd at the top of the boot partition"; else fail "B: initrd path wrong"; fi
if printf '%s' "$OUT_B" | grep -q "# access /dev/sda1"; then pass "B: GRUB reads the kernel from the boot partition"; else fail "B: kernel read from the wrong partition"; fi

# --- Scenario C: older setup, links only at the filesystem root -------------
export MOCK_BOOT_REL="/boot"
export GRUB_DEVICE_BOOT="/dev/sda2"
OUT_C="$(SYSROOT="$WORK/sys-root-links" run_script C)"
if printf '%s' "$OUT_C" | grep -q "linux	/vmlinuz root="; then pass "C: root-level kernel link used"; else fail "C: root-level kernel link not used"; fi
if printf '%s' "$OUT_C" | grep -q "initrd	/initrd.img"; then pass "C: root-level initrd link used"; else fail "C: root-level initrd link not used"; fi

# --- Scenario D: no kernel links -> leave the menu to Ubuntu's entries -------
OUT_D="$(SYSROOT="$WORK/sys-none" run_script D)"
if ! printf '%s' "$OUT_D" | grep -q "menuentry"; then pass "D: no entries without a kernel (Windows never becomes the default)"; else fail "D: entries emitted without a kernel"; fi

# --- grub-script-check on the generated fragment ---------------------------
if command -v grub-script-check >/dev/null 2>&1; then
  if grub-script-check "$WORK/out_a.cfg" 2>/dev/null; then
    pass "A: output passes grub-script-check"
  else
    fail "A: grub-script-check rejected output"
  fi
else
  echo "SKIP grub-script-check not installed" >&2
fi

# --- Against the REAL lib if present ---------------------------------------
if [ -r /usr/share/grub/grub-mkconfig_lib ]; then
  OUT_REAL="$(env PATH="$WORK/bin:$PATH" pkgdatadir=/usr/share/grub \
      grub_probe="$WORK/bin/grub-probe" \
      GRUB_DEVICE="/dev/sda2" GRUB_DEVICE_UUID="DEAD-BEEF" GRUB_DEVICE_BOOT="/dev/sda2" \
      GRUB_CMDLINE_LINUX="" GRUB_CMDLINE_LINUX_DEFAULT="quiet splash" \
      CHATCOMPUTER_SYSROOT="$WORK/sys-boot-links" bash "$SCRIPT" 2>/dev/null)"
  if printf '%s' "$OUT_REAL" | grep -q "menuentry 'Chat Computer'"; then pass "real-lib: Chat Computer entry present"; else fail "real-lib: Chat Computer entry missing"; fi
  if printf '%s' "$OUT_REAL" | grep -q "menuentry 'Windows'"; then pass "real-lib: Windows entry present"; else fail "real-lib: Windows entry missing"; fi
else
  echo "SKIP real-lib run (grub-mkconfig_lib not installed)" >&2
fi

exit "$FAILED"
