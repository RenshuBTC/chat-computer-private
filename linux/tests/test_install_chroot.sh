#!/bin/bash
# Real install test: build a noble chroot, run install-chatcomputer.sh
# --from-installer --skip-models inside it, verify the result, and prove
# idempotency by running it twice.
#
# Heavy (debootstrap + apt). Skips gracefully unless:
#   * running as root,
#   * debootstrap is available,
#   * CHATCOMPUTER_RUN_CHROOT=1 is set (opt-in, because it downloads a lot).
# run.sh sets CHATCOMPUTER_RUN_CHROOT=1 when it detects it can.
#
# Env:
#   CHATCOMPUTER_CHROOT_DIR   where to build (default: a mktemp dir)
#   CHATCOMPUTER_KEEP_CHROOT  if 1, do not delete the chroot on exit
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/.." && pwd)"
APP_PARENT="$(cd "$REPO/.." && pwd)"   # the 'app' dir that contains linux/
FAILED=0
pass() { printf 'PASS %s\n' "$1"; }
fail() { printf 'FAIL %s\n' "$1"; FAILED=1; }

if [ "${CHATCOMPUTER_RUN_CHROOT:-0}" != "1" ]; then
  echo "SKIP chroot install test (set CHATCOMPUTER_RUN_CHROOT=1 to enable)" >&2
  exit 0
fi
if [ "$(id -u)" -ne 0 ]; then echo "SKIP chroot install test: not root" >&2; exit 0; fi
if ! command -v debootstrap >/dev/null 2>&1; then echo "SKIP chroot install test: no debootstrap" >&2; exit 0; fi

C="${CHATCOMPUTER_CHROOT_DIR:-$(mktemp -d)}/noble"
mkdir -p "$C"
cleanup() {
  for m in "$C/dev/pts" "$C/dev" "$C/sys" "$C/proc"; do
    mountpoint -q "$m" 2>/dev/null && umount -lf "$m" 2>/dev/null || true
  done
  if [ "${CHATCOMPUTER_KEEP_CHROOT:-0}" != "1" ]; then rm -rf "$(dirname "$C")"; fi
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

# Copy the repo into /opt/chatcomputer/app and provide a THROWAWAY requirements file
# (never committed to the repo root). regipy is what the bt importer needs.
mkdir -p "$C/opt/chatcomputer/app"
cp -a "$APP_PARENT/linux" "$APP_PARENT/chatcomputer" "$C/opt/chatcomputer/app/"
cat > "$C/opt/chatcomputer/app/requirements-linux.txt" <<'EOF'
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
mkdir -p "$C/etc/chatcomputer" "$C/etc/systemd/system/multi-user.target.wants"
echo testuser > "$C/etc/chatcomputer/install-user"
cp "$C/opt/chatcomputer/app/linux/firstboot/chatcomputer-finish-install.service" "$C/etc/systemd/system/"
ln -sf ../chatcomputer-finish-install.service "$C/etc/systemd/system/multi-user.target.wants/chatcomputer-finish-install.service"

echo "=== installer run #1 ===" >&2
if runin 'bash /opt/chatcomputer/app/linux/install-chatcomputer.sh --user testuser --from-installer --skip-models' >/tmp/ci_install1.log 2>&1; then
  pass "installer run #1 exited 0"
else
  fail "installer run #1 non-zero"; tail -20 /tmp/ci_install1.log >&2
fi

# --- File presence ---------------------------------------------------------
for f in \
  "/etc/systemd/system/getty@tty1.service.d/10-chatcomputer-autologin.conf" \
  "/home/testuser/.bash_profile" "/home/testuser/.xinitrc" \
  "/etc/X11/Xwrapper.config" \
  "/etc/wireplumber/bluetooth.lua.d/51-chatcomputer-bluetooth.lua" \
  "/etc/wireplumber/policy.lua.d/51-chatcomputer-policy.lua" \
  "/etc/NetworkManager/conf.d/10-chatcomputer-manage-all.conf" \
  "/etc/netplan/99-chatcomputer-nm.yaml" \
  "/etc/systemd/system/chatcomputer-windows-mount.service" \
  "/usr/local/libexec/chatcomputer/chatcomputer-root" \
  "/usr/local/libexec/chatcomputer/bt-import-windows-keys.py" \
  "/etc/grub.d/07_chatcomputer" "/etc/default/grub.d/99-chatcomputer.cfg" \
  "/etc/apt/apt.conf.d/52chatcomputer-unattended-upgrades" \
  "/etc/sudoers.d/chatcomputer" \
  "/var/lib/chatcomputer/base-installed"; do
  [ -e "$C$f" ] && pass "file present: $f" || fail "file MISSING: $f"
done
# The list of Chat Computer's own packages, which the root helper never removes.
grep -qx 'gir1.2-webkit2-4.1' "$C/etc/chatcomputer/base-packages" 2>/dev/null && grep -qx 'pipewire' "$C/etc/chatcomputer/base-packages" \
  && pass "Chat Computer's own packages are listed for the root helper" || fail "/etc/chatcomputer/base-packages missing or incomplete"
# WirePlumber 0.4 runs the Lua files of the X.lua.d folders its own package has (and the same names in /etc);
# a Lua file anywhere else is never read.
for d in bluetooth.lua.d policy.lua.d; do
  [ -d "$C/usr/share/wireplumber/$d" ] && pass "WirePlumber's package reads $d, where Chat Computer's settings are" \
    || fail "the installed WirePlumber has no $d: Chat Computer's settings there are ignored"
done
[ ! -e "$C/etc/systemd/system/multi-user.target.wants/chatcomputer-finish-install.service" ] \
  && pass "first-start safety net turned off after a complete install" \
  || fail "first-start safety net still enabled after a complete install"

# --- Autologin rendered with the real user ---------------------------------
grep -q 'autologin testuser' "$C/etc/systemd/system/getty@tty1.service.d/10-chatcomputer-autologin.conf" \
  && pass "autologin drop-in names testuser" || fail "autologin drop-in not rendered"
grep -q -- '--noissue' "$C/etc/systemd/system/getty@tty1.service.d/10-chatcomputer-autologin.conf" \
  && pass "no Ubuntu banner on tty1" || fail "tty1 still shows the Ubuntu banner"
[ -f "$C/home/testuser/.hushlogin" ] && [ ! -s "$C/home/testuser/.hushlogin" ] \
  && pass "no welcome text at login (.hushlogin)" || fail ".hushlogin missing from the home folder"

# --- sudoers validated by the REAL visudo ----------------------------------
if chroot "$C" visudo -cf /etc/sudoers.d/chatcomputer >/dev/null 2>&1; then
  pass "sudoers passes visudo -cf"
else
  fail "sudoers rejected by visudo"
fi
grep -q '^testuser ALL=(root) NOPASSWD: /usr/local/libexec/chatcomputer/chatcomputer-root$' "$C/etc/sudoers.d/chatcomputer" \
  && pass "sudoers grants exactly the root helper" || fail "sudoers content wrong"

# --- groups ----------------------------------------------------------------
g="$(chroot "$C" id -nG testuser)"
ok=1; for want in audio video input bluetooth netdev; do
  printf '%s' "$g" | tr ' ' '\n' | grep -qx "$want" || { ok=0; fail "user not in group $want"; }
done
[ "$ok" = 1 ] && pass "user in audio/video/input/bluetooth/netdev"

# --- venv + imports --------------------------------------------------------
[ -x "$C/opt/chatcomputer/venv/bin/python" ] && pass "venv python present" || fail "venv python missing"
# Install regipy (requirements) and check the critical imports.
runin '/opt/chatcomputer/venv/bin/pip install -r /opt/chatcomputer/app/requirements-linux.txt' >/tmp/ci_pip.log 2>&1 || true
if runin '/opt/chatcomputer/venv/bin/python -c "import gi; gi.require_version(\"WebKit2\",\"4.1\"); gi.require_version(\"Gtk\",\"3.0\"); from gi.repository import WebKit2, Gtk"' >/dev/null 2>&1; then
  pass "venv imports gi + WebKit2 4.1 + Gtk 3.0"
else
  fail "venv cannot import WebKit2/Gtk"
fi
if runin '/opt/chatcomputer/venv/bin/python -c "import regipy"' >/dev/null 2>&1; then
  pass "venv imports regipy"
else
  fail "venv missing regipy (bt import would fail)"
fi
# The session starts `python -m chatcomputer run` from the home folder, not from the app folder.
if runin 'cd /home && /opt/chatcomputer/venv/bin/python -c "import chatcomputer.config, chatcomputer.__main__"' >/dev/null 2>&1; then
  pass "venv loads Chat Computer from any folder"
else
  fail "venv can't load Chat Computer outside /opt/chatcomputer/app (the session would crash)"
fi

# --- services wired --------------------------------------------------------
[ -L "$C/etc/systemd/system/multi-user.target.wants/chatcomputer-windows-mount.service" ] \
  && pass "chatcomputer-windows-mount wired into multi-user.target" \
  || fail "chatcomputer-windows-mount not enabled"
[ -L "$C/etc/systemd/system/multi-user.target.wants/NetworkManager.service" ] \
  && pass "NetworkManager enabled" || fail "NetworkManager not enabled"
if find "$C/etc/systemd/system" -path '*.wants/*' -name 'systemd-networkd*' | grep -q .; then
  fail "systemd-networkd still enabled (its wait-online would delay every start by 2 minutes)"
else
  pass "systemd-networkd and its wait-online are off"
fi
case "$(chroot "$C" readlink -f /etc/alternatives/default.plymouth 2>/dev/null)" in
  /usr/share/plymouth/themes/chatcomputer/chatcomputer.plymouth) pass "boot splash is Chat Computer's" ;;
  *) fail "boot splash is not Chat Computer's" ;;
esac

# --- ufw policy ------------------------------------------------------------
grep -q 'DEFAULT_INPUT_POLICY="DROP"' "$C/etc/default/ufw" && pass "ufw default deny incoming" || fail "ufw input policy not DROP"
grep -q 'DEFAULT_OUTPUT_POLICY="ACCEPT"' "$C/etc/default/ufw" && pass "ufw default allow outgoing" || fail "ufw output policy not ACCEPT"

# --- no SSH server ---------------------------------------------------------
chroot "$C" dpkg -s openssh-server >/dev/null 2>&1 && fail "openssh-server present (should be absent)" || pass "no SSH server installed"

# --- root helper runs (installed copy) -------------------------------------
if chroot "$C" /usr/local/libexec/chatcomputer/chatcomputer-root status | "${CHATCOMPUTER_TEST_PY:-python3}" -c 'import sys,json; json.load(sys.stdin)' >/dev/null 2>&1; then
  pass "installed root helper status returns valid JSON"
else
  fail "installed root helper status failed"
fi

# --- bt importer via the installed venv + synthetic hive -------------------
cp "$HERE/make_system_hive.py" "$C/root/mk.py"
if runin '/opt/chatcomputer/venv/bin/python /root/mk.py /root/SYSTEM && mkdir -p /var/lib/bluetooth/0A:1B:2C:3D:4E:5F && /opt/chatcomputer/venv/bin/python -I /usr/local/libexec/chatcomputer/bt-import-windows-keys.py --hive /root/SYSTEM --bluetooth-dir /var/lib/bluetooth' >/tmp/ci_bt.log 2>&1; then
  if grep -q 'Key=FEDCBA9876543210FEDCBA9876543210' "$C/var/lib/bluetooth/0A:1B:2C:3D:4E:5F/C0:FF:EE:12:34:56/info" 2>/dev/null; then
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
if chroot "$C" bash -c 'pkgdatadir=/usr/share/grub GRUB_DEVICE=/dev/sda2 GRUB_DEVICE_UUID=TEST GRUB_DEVICE_BOOT=/dev/sda2 GRUB_CMDLINE_LINUX="" GRUB_CMDLINE_LINUX_DEFAULT="quiet splash" /etc/grub.d/07_chatcomputer 2>/dev/null' > "$C/tmp/g07.out" 2>&1; then
  if grep -q "menuentry 'Chat Computer'" "$C/tmp/g07.out" && grep -q "menuentry 'Windows'" "$C/tmp/g07.out"; then
    pass "07_chatcomputer (real lib) emits Chat Computer + Windows entries"
  else
    fail "07_chatcomputer output missing entries"
  fi
  if chroot "$C" grub-script-check < "$C/tmp/g07.out" 2>/dev/null; then pass "07_chatcomputer output passes grub-script-check"; else fail "07_chatcomputer output fails grub-script-check"; fi
else
  fail "07_chatcomputer failed under real lib"
fi

# --- Idempotency: run #2, assert no warnings + stable files ----------------
# As an earlier version left them: Lua settings in wireplumber.conf.d, which WirePlumber 0.4 doesn't read.
mkdir -p "$C/etc/wireplumber/wireplumber.conf.d"
for f in 51-chatcomputer-bluetooth.lua 51-chatcomputer-policy.lua; do : > "$C/etc/wireplumber/wireplumber.conf.d/$f"; done
pre="$(sha256sum "$C/etc/bluetooth/main.conf" "$C/etc/sudoers.d/chatcomputer" "$C/home/testuser/.bash_profile" 2>/dev/null)"
echo "=== installer run #2 (idempotency) ===" >&2
runin 'bash /opt/chatcomputer/app/linux/install-chatcomputer.sh --user testuser --from-installer --skip-models' >/tmp/ci_install2.log 2>&1
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
post="$(sha256sum "$C/etc/bluetooth/main.conf" "$C/etc/sudoers.d/chatcomputer" "$C/home/testuser/.bash_profile" 2>/dev/null)"
[ "$pre" = "$post" ] && pass "idempotent: managed files unchanged on re-run" || fail "files changed on re-run"
ls "$C"/etc/wireplumber/wireplumber.conf.d/*.lua >/dev/null 2>&1 \
  && fail "Lua files left in wireplumber.conf.d" || pass "an earlier version's unread Lua files are gone"
# BlueZ managed block appears exactly once.
n="$(grep -c 'Chat Computer managed block >>>' "$C/etc/bluetooth/main.conf" 2>/dev/null || echo 0)"
[ "$n" = "1" ] && pass "BlueZ managed block present exactly once" || fail "BlueZ managed block duplicated ($n)"

exit "$FAILED"
