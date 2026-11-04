#!/bin/bash
# install-chatcomputer.sh --dry-run on both systems it installs on: Ubuntu (a PC) and Fedora Asahi Remix (a Mac).
# Checks each takes its own steps and none of the other's, and that every file it would install is in the
# repo. Changes nothing (dry run), so it runs anywhere; the real installs are tested by
# test_install_chroot.sh (Ubuntu) and the Mac's emulated first start.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/.." && pwd)"   # linux/
FAILED=0
pass() { printf 'PASS %s\n' "$1"; }
fail() { printf 'FAIL %s\n' "$1"; FAILED=1; }

W="$(mktemp -d)"
trap 'rm -rf "$W"' EXIT

for d in ubuntu fedora; do
  bash "$REPO/install-chatcomputer.sh" --user testuser --dry-run --skip-models --distro "$d" > "$W/$d" 2>&1
  rc=$?
  [ "$rc" -eq 0 ] && pass "$d: dry run finishes" || { fail "$d: exit status $rc"; tail -5 "$W/$d"; }
  grep -q 'missing source file' "$W/$d" && fail "$d: $(grep 'missing source file' "$W/$d" | head -3)" \
    || pass "$d: every file it installs is in the repo"
  grep -q 'distro='"$d" "$W/$d" && pass "$d: says which system it is installing on" || fail "$d: no distro line"
done

# has FILE PATTERN WHAT / lacks FILE PATTERN WHAT
has()   { grep -qE -- "$2" "$W/$1" && pass "$1: $3" || fail "$1: $3 (no match for $2)"; }
lacks() { grep -qE -- "$2" "$W/$1" && fail "$1: $3 (found $2)" || pass "$1: $3"; }

# Ubuntu (a PC).
has   ubuntu '\[dry-run\] apt-get install -y --no-install-recommends .*xserver-xorg-core' "packages with apt"
has   ubuntu 'cryptsetup-bin' "BitLocker support (cryptsetup)"
has   ubuntu 'Xwrapper.config' "rootless X through Xorg.wrap"
has   ubuntu '51-chatcomputer-bluetooth\.lua' "WirePlumber 0.4 settings (Lua)"
has   ubuntu '99-chatcomputer-nm\.yaml' "netplan hands the network to NetworkManager"
has   ubuntu 'chatcomputer-windows-mount\.service' "mounts Windows' drive"
has   ubuntu '07_chatcomputer' "GRUB entries for Chat Computer and Windows"
has   ubuntu 'ufw default deny incoming' "ufw firewall"
has   ubuntu '52chatcomputer-unattended-upgrades' "automatic security updates (unattended-upgrades)"
lacks ubuntu 'dnf |grubby|grub2-mkconfig|firewalld|firewall-offline|dnf5-automatic|asahi-bless|/Mac' "nothing of the Mac's"

# Fedora (a Mac).
has   fedora '\[dry-run\] dnf install -y --setopt=install_weak_deps=False .*xorg-x11-server-Xorg' "packages with dnf, no weak dependencies"
has   fedora 'dnf install .*asahi-bless' "asahi-bless, to restart into macOS"
has   fedora 'dnf install .*exfatprogs' "exFAT, for the drive macOS shares"
has   fedora 'dnf install .*webkit2gtk4\.1' "WebKitGTK 4.1 for Chat Computer's screen"
has   fedora 'dnf install .*gobject-introspection' "cairo's typelib, which GTK's needs (weak dependencies are off)"
has   fedora '51-chatcomputer-bluetooth\.conf' "WirePlumber 0.5 settings (SPA-JSON)"
has   fedora 'no Chat Computer drive recorded|PARTUUID=.*/Mac' "looks for the drive macOS shares"
has   fedora 'hidden menu, quiet rhgb' "GRUB starts Chat Computer without its menu"
has   fedora 'firewalld: remove ssh and cockpit' "firewalld with nothing extra open"
has   fedora 'systemctl enable firewalld\.service' "firewalld on"
has   fedora 'automatic\.conf' "dnf automatic settings"
has   fedora 'systemctl enable dnf5-automatic\.timer' "automatic security updates (dnf5-automatic)"
has   fedora 'systemctl enable NetworkManager\.service' "NetworkManager on"
has   fedora 'getty@tty1' "signs in on the first console by itself"
has   fedora 'sudoers' "the root helper's sudo rule"
lacks fedora 'apt-get|netplan|ufw|update-grub|07_chatcomputer|Xwrapper|chatcomputer-windows-mount|\.lua |ubuntu-drivers|unattended-upgrades' \
  "nothing of the PC's"

# In the installer the voice is downloaded then, so Chat Computer can talk the first time it starts.
bash "$REPO/install-chatcomputer.sh" --user testuser --dry-run --from-installer --distro ubuntu > "$W/installer" 2>&1
has   installer 'download the voice models into /home/testuser/ChatComputer/models' "the voice, ready for the first start"

# The two package lists don't mention each other's names.
for p in xserver-xorg-core libspa-0.2-bluetooth network-manager ntfs-3g efibootmgr fonts-noto-core; do
  grep -q "dnf install .*[[:space:]]$p\([[:space:]]\|$\)" "$W/fedora" && fail "fedora: Ubuntu's package $p in the dnf list"
done
pass "fedora: no Ubuntu package names in the dnf list"

# Which system it is, from os-release, when --distro isn't given.
for case in "fedora-asahi-remix|fedora|fedora" "fedora||fedora" "ubuntu|debian|ubuntu"; do
  IFS='|' read -r id like want <<EOF
$case
EOF
  printf 'NAME=x\nID=%s\nID_LIKE="%s"\n' "$id" "$like" > "$W/os-release"
  CHATCOMPUTER_OS_RELEASE="$W/os-release" bash "$REPO/install-chatcomputer.sh" --user testuser --dry-run --skip-models > "$W/detect" 2>&1
  grep -q "distro=$want" "$W/detect" && pass "ID=$id ID_LIKE=$like is $want" || fail "ID=$id ID_LIKE=$like: $(grep -o 'distro=[a-z]*' "$W/detect")"
done

# Bad arguments.
bash "$REPO/install-chatcomputer.sh" --user testuser --dry-run --distro debian >/dev/null 2>&1
[ $? -eq 2 ] && pass "refuses an unknown --distro" || fail "accepted --distro debian"
bash "$REPO/install-chatcomputer.sh" --user 'Bad User' --dry-run >/dev/null 2>&1
[ $? -eq 2 ] && pass "refuses a bad user name" || fail "accepted a bad user name"

exit "$FAILED"
