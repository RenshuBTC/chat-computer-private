#!/bin/bash
# Real install test: build a noble chroot, run install-claudeos.sh
# --from-installer --skip-models inside it, verify the result, and prove
# idempotency by running it twice.
#
# Heavy (debootstrap + apt). Skips gracefully unless:
#   * running as root,
#   * debootstrap is available,
#   * CLAUDEOS_RUN_CHROOT=1 is set (opt-in, because it downloads a lot).
# run.sh sets CLAUDEOS_RUN_CHROOT=1 when it detects it can.
#
# Env:
#   CLAUDEOS_CHROOT_DIR   where to build (default: a mktemp dir)
#   CLAUDEOS_KEEP_CHROOT  if 1, do not delete the chroot on exit
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/.." && pwd)"
APP_PARENT="$(cd "$REPO/.." && pwd)"   # the 'app' dir that contains linux/
FAILED=0
pass() { printf 'PASS %s\n' "$1"; }
fail() { printf 'FAIL %s\n' "$1"; FAILED=1; }

if [ "${CLAUDEOS_RUN_CHROOT:-0}" != "1" ]; then
  echo "SKIP chroot install test (set CLAUDEOS_RUN_CHROOT=1 to enable)" >&2
  exit 0
fi
if [ "$(id -u)" -ne 0 ]; then echo "SKIP chroot install test: not root" >&2; exit 0; fi
if ! command -v debootstrap >/dev/null 2>&1; then echo "SKIP chroot install test: no debootstrap" >&2; exit 0; fi

C="${CLAUDEOS_CHROOT_DIR:-$(mktemp -d)}/noble"
mkdir -p "$C"
cleanup() {
  for m in "$C/dev/pts" "$C/dev" "$C/sys" "$C/proc"; do
    mountpoint -q "$m" 2>/dev/null && umount -lf "$m" 2>/dev/null || true
  done
  if [ "${CLAUDEOS_KEEP_CHROOT:-0}" != "1" ]; then rm -rf "$(dirname "$C")"; fi
}
trap cleanup EXIT

echo "building noble chroot at $C ..." >&2
if ! debootstrap --variant=minbase --include=ca-certificates,apt-utils \
     --components=main,universe noble "$C" http://archive.ubuntu.com/ubuntu/ >/dev/null 2>&1; then
  fail "debootstrap failed"; exit "$FAILED"
fi
pass "debootstrap built noble chroot"

# apt sources (universe) + proxy + CA (test environment uses an HTTPS proxy).
cat > "$C/etc/apt/sources.list" <<'EOF'
deb http://archive.ubuntu.com/ubuntu noble main universe
deb http://archive.ubuntu.com/ubuntu noble-updates main universe
deb http://security.ubuntu.com/ubuntu noble-security main universe
EOF
if [ -n "${https_proxy:-}" ]; then
  echo "Acquire::https::Proxy \"$https_proxy\";" > "$C/etc/apt/apt.conf.d/01proxy"
fi
cp /etc/resolv.conf "$C/etc/resolv.conf" 2>/dev/null || true
# Make the proxy CA reachable at the same path inside the chroot so pip works.
if [ -f /root/.ccr/ca-bundle.crt ]; then
  mkdir -p "$C/root/.ccr"; cp /root/.ccr/ca-bundle.crt "$C/root/.ccr/ca-bundle.crt"
fi

# Copy the repo into /opt/claudeos/app and provide a THROWAWAY requirements file
# (never committed to the repo root). regipy is what the bt importer needs.
mkdir -p "$C/opt/claudeos/app"
cp -a "$APP_PARENT/linux" "$APP_PARENT/claudeos" "$C/opt/claudeos/app/"
cat > "$C/opt/claudeos/app/requirements-linux.txt" <<'EOF'
regipy==6.5.0
EOF

# Bind mounts.
mount -t proc proc "$C/proc" 2>/dev/null || mount --bind /proc "$C/proc"
mount --rbind /sys "$C/sys"; mount --rbind /dev "$C/dev"

# Create the user subiquity would have created before late-commands.
chroot "$C" bash -c 'id testuser >/dev/null 2>&1 || useradd -m -s /bin/bash testuser'
chroot "$C" bash -c 'export https_proxy='"${https_proxy:-}"'; DEBIAN_FRONTEND=noninteractive apt-get update >/dev/null 2>&1' || true

runin() {
  chroot "$C" bash -c "export https_proxy='${https_proxy:-}' http_proxy= PIP_CERT=/root/.ccr/ca-bundle.crt REQUESTS_CA_BUNDLE=/root/.ccr/ca-bundle.crt DEBIAN_FRONTEND=noninteractive; $*"
}

# The first-start safety net that the USB stick's finish.sh enables before running the installer
# (enabling a unit is just this symlink).
mkdir -p "$C/etc/claudeos" "$C/etc/systemd/system/multi-user.target.wants"
echo testuser > "$C/etc/claudeos/install-user"
cp "$C/opt/claudeos/app/linux/firstboot/claudeos-finish-install.service" "$C/etc/systemd/system/"
ln -sf ../claudeos-finish-install.service "$C/etc/systemd/system/multi-user.target.wants/claudeos-finish-install.service"

echo "=== installer run #1 ===" >&2
if runin 'bash /opt/claudeos/app/linux/install-claudeos.sh --user testuser --from-installer --skip-models' >/tmp/ci_install1.log 2>&1; then
  pass "installer run #1 exited 0"
else
  fail "installer run #1 non-zero"; tail -20 /tmp/ci_install1.log >&2
fi

# --- File presence ---------------------------------------------------------
for f in \
  "/etc/systemd/system/getty@tty1.service.d/10-claudeos-autologin.conf" \
  "/home/testuser/.bash_profile" "/home/testuser/.xinitrc" \
  "/etc/X11/Xwrapper.config" \
  "/etc/wireplumber/wireplumber.conf.d/51-claudeos-bluetooth.lua" \
  "/etc/wireplumber/wireplumber.conf.d/51-claudeos-policy.lua" \
  "/etc/NetworkManager/conf.d/10-claudeos-manage-all.conf" \
  "/etc/netplan/99-claudeos-nm.yaml" \
  "/etc/systemd/system/claudeos-windows-mount.service" \
  "/usr/local/libexec/claudeos/claudeos-root" \
  "/usr/local/libexec/claudeos/bt-import-windows-keys.py" \
  "/etc/grub.d/07_claudeos" "/etc/default/grub.d/99-claudeos.cfg" \
  "/etc/apt/apt.conf.d/52claudeos-unattended-upgrades" \
  "/etc/sudoers.d/claudeos" \
  "/var/lib/claudeos/base-installed"; do
  [ -e "$C$f" ] && pass "file present: $f" || fail "file MISSING: $f"
done
[ ! -e "$C/etc/systemd/system/multi-user.target.wants/claudeos-finish-install.service" ] \
  && pass "first-start safety net turned off after a complete install" \
  || fail "first-start safety net still enabled after a complete install"

# --- Autologin rendered with the real user ---------------------------------
grep -q 'autologin testuser' "$C/etc/systemd/system/getty@tty1.service.d/10-claudeos-autologin.conf" \
  && pass "autologin drop-in names testuser" || fail "autologin drop-in not rendered"
grep -q -- '--noissue' "$C/etc/systemd/system/getty@tty1.service.d/10-claudeos-autologin.conf" \
  && pass "no Ubuntu banner on tty1" || fail "tty1 still shows the Ubuntu banner"
[ -f "$C/home/testuser/.hushlogin" ] && [ ! -s "$C/home/testuser/.hushlogin" ] \
  && pass "no welcome text at login (.hushlogin)" || fail ".hushlogin missing from the home folder"

# --- sudoers validated by the REAL visudo ----------------------------------
if chroot "$C" visudo -cf /etc/sudoers.d/claudeos >/dev/null 2>&1; then
  pass "sudoers passes visudo -cf"
else
  fail "sudoers rejected by visudo"
fi
grep -q '^testuser ALL=(root) NOPASSWD: /usr/local/libexec/claudeos/claudeos-root$' "$C/etc/sudoers.d/claudeos" \
  && pass "sudoers grants exactly the root helper" || fail "sudoers content wrong"

# --- groups ----------------------------------------------------------------
g="$(chroot "$C" id -nG testuser)"
ok=1; for want in audio video input bluetooth netdev; do
  printf '%s' "$g" | tr ' ' '\n' | grep -qx "$want" || { ok=0; fail "user not in group $want"; }
done
[ "$ok" = 1 ] && pass "user in audio/video/input/bluetooth/netdev"

# --- venv + imports --------------------------------------------------------
[ -x "$C/opt/claudeos/venv/bin/python" ] && pass "venv python present" || fail "venv python missing"
# Install regipy (requirements) and check the critical imports.
runin '/opt/claudeos/venv/bin/pip install -r /opt/claudeos/app/requirements-linux.txt' >/tmp/ci_pip.log 2>&1 || true
if runin '/opt/claudeos/venv/bin/python -c "import gi; gi.require_version(\"WebKit2\",\"4.1\"); gi.require_version(\"Gtk\",\"3.0\"); from gi.repository import WebKit2, Gtk"' >/dev/null 2>&1; then
  pass "venv imports gi + WebKit2 4.1 + Gtk 3.0"
else
  fail "venv cannot import WebKit2/Gtk"
fi
if runin '/opt/claudeos/venv/bin/python -c "import regipy"' >/dev/null 2>&1; then
  pass "venv imports regipy"
else
  fail "venv missing regipy (bt import would fail)"
fi
# The session starts `python -m claudeos run` from the home folder, not from the app folder.
if runin 'cd /home && /opt/claudeos/venv/bin/python -c "import claudeos.config, claudeos.__main__"' >/dev/null 2>&1; then
  pass "venv loads ClaudeOS from any folder"
else
  fail "venv can't load ClaudeOS outside /opt/claudeos/app (the session would crash)"
fi

# --- services wired --------------------------------------------------------
[ -L "$C/etc/systemd/system/multi-user.target.wants/claudeos-windows-mount.service" ] \
  && pass "claudeos-windows-mount wired into multi-user.target" \
  || fail "claudeos-windows-mount not enabled"
[ -L "$C/etc/systemd/system/multi-user.target.wants/NetworkManager.service" ] \
  && pass "NetworkManager enabled" || fail "NetworkManager not enabled"
if find "$C/etc/systemd/system" -path '*.wants/*' -name 'systemd-networkd*' | grep -q .; then
  fail "systemd-networkd still enabled (its wait-online would delay every start by 2 minutes)"
else
  pass "systemd-networkd and its wait-online are off"
fi
case "$(chroot "$C" readlink -f /etc/alternatives/default.plymouth 2>/dev/null)" in
  /usr/share/plymouth/themes/claudeos/claudeos.plymouth) pass "boot splash is ClaudeOS's" ;;
  *) fail "boot splash is not ClaudeOS's" ;;
esac

# --- ufw policy ------------------------------------------------------------
grep -q 'DEFAULT_INPUT_POLICY="DROP"' "$C/etc/default/ufw" && pass "ufw default deny incoming" || fail "ufw input policy not DROP"
grep -q 'DEFAULT_OUTPUT_POLICY="ACCEPT"' "$C/etc/default/ufw" && pass "ufw default allow outgoing" || fail "ufw output policy not ACCEPT"

# --- no SSH server ---------------------------------------------------------
chroot "$C" dpkg -s openssh-server >/dev/null 2>&1 && fail "openssh-server present (should be absent)" || pass "no SSH server installed"

# --- root helper runs (installed copy) -------------------------------------
if chroot "$C" /usr/local/libexec/claudeos/claudeos-root status | "${CLAUDEOS_TEST_PY:-python3}" -c 'import sys,json; json.load(sys.stdin)' >/dev/null 2>&1; then
  pass "installed root helper status returns valid JSON"
else
  fail "installed root helper status failed"
fi

# --- bt importer via the installed venv + synthetic hive -------------------
cp "$HERE/make_system_hive.py" "$C/root/mk.py"
if runin '/opt/claudeos/venv/bin/python /root/mk.py /root/SYSTEM && mkdir -p /var/lib/bluetooth/D4:6D:6D:97:62:9B && /opt/claudeos/venv/bin/python -I /usr/local/libexec/claudeos/bt-import-windows-keys.py --hive /root/SYSTEM --bluetooth-dir /var/lib/bluetooth' >/tmp/ci_bt.log 2>&1; then
  if grep -q 'Key=C486D66A8779AFBE72FA81C749DC580C' "$C/var/lib/bluetooth/D4:6D:6D:97:62:9B/C2:9E:1D:E2:3D:A5/info" 2>/dev/null; then
    pass "bt importer (venv) wrote correct BlueZ info in chroot"
  else
    fail "bt importer did not write expected info"; cat /tmp/ci_bt.log >&2
  fi
else
  fail "bt importer failed in chroot"; cat /tmp/ci_bt.log >&2
fi

# --- grub 07 under the real lib --------------------------------------------
chroot "$C" bash -c 'export https_proxy='"${https_proxy:-}"' DEBIAN_FRONTEND=noninteractive; apt-get install -y --no-install-recommends grub-common >/dev/null 2>&1' || true
chroot "$C" bash -c 'mkdir -p /boot && touch /boot/vmlinuz /boot/initrd.img' 2>/dev/null || true   # Ubuntu 24.04 keeps the links in /boot
if chroot "$C" bash -c 'pkgdatadir=/usr/share/grub GRUB_DEVICE=/dev/sda2 GRUB_DEVICE_UUID=TEST GRUB_DEVICE_BOOT=/dev/sda2 GRUB_CMDLINE_LINUX="" GRUB_CMDLINE_LINUX_DEFAULT="quiet splash" /etc/grub.d/07_claudeos 2>/dev/null' > "$C/tmp/g07.out" 2>&1; then
  if grep -q "menuentry 'ClaudeOS'" "$C/tmp/g07.out" && grep -q "menuentry 'Windows'" "$C/tmp/g07.out"; then
    pass "07_claudeos (real lib) emits ClaudeOS + Windows entries"
  else
    fail "07_claudeos output missing entries"
  fi
  if chroot "$C" grub-script-check < "$C/tmp/g07.out" 2>/dev/null; then pass "07_claudeos output passes grub-script-check"; else fail "07_claudeos output fails grub-script-check"; fi
else
  fail "07_claudeos failed under real lib"
fi

# --- Idempotency: run #2, assert no warnings + stable files ----------------
pre="$(sha256sum "$C/etc/bluetooth/main.conf" "$C/etc/sudoers.d/claudeos" "$C/home/testuser/.bash_profile" 2>/dev/null)"
echo "=== installer run #2 (idempotency) ===" >&2
runin 'bash /opt/claudeos/app/linux/install-claudeos.sh --user testuser --from-installer --skip-models' >/tmp/ci_install2.log 2>&1
rc=$?
[ "$rc" -eq 0 ] && pass "installer run #2 exited 0" || fail "installer run #2 non-zero"
# No WARNING lines except those that are inherent to running inside a chroot
# with no real boot disk / running systemd / proxy CA (grub-mkconfig, netplan,
# pip-over-proxy). Those are environment artifacts, not install defects.
benign='ca-cert|TLS| pip |pip |model|grub-mkconfig|update-grub|grub-set-default|netplan|start .* now|could not start|update-initramfs'
if grep -i 'WARNING' /tmp/ci_install2.log | grep -viE "$benign" | grep -q .; then
  fail "installer run #2 emitted unexpected warnings"
  grep -i warning /tmp/ci_install2.log | grep -viE "$benign" >&2
else
  pass "installer run #2 had no unexpected (non-chroot) warnings"
fi
post="$(sha256sum "$C/etc/bluetooth/main.conf" "$C/etc/sudoers.d/claudeos" "$C/home/testuser/.bash_profile" 2>/dev/null)"
[ "$pre" = "$post" ] && pass "idempotent: managed files unchanged on re-run" || fail "files changed on re-run"
# BlueZ managed block appears exactly once.
n="$(grep -c 'ClaudeOS managed block >>>' "$C/etc/bluetooth/main.conf" 2>/dev/null || echo 0)"
[ "$n" = "1" ] && pass "BlueZ managed block present exactly once" || fail "BlueZ managed block duplicated ($n)"

exit "$FAILED"
