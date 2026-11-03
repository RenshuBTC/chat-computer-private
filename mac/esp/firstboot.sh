#!/bin/sh
# Chat Computer's first start on a Mac (runs on the Mac, under Linux, as root).
#
# The Asahi installer has just put Fedora Asahi Remix (its Minimal edition) next to macOS, driven by Chat Computer
# Setup in macOS, which left this program and the person's settings on Chat Computer's EFI partition
# (/boot/efi/chatcomputer) and started this once in place of Fedora's usual first start (grub-firststart.cfg). It
# makes the person's account, joins their Wi-Fi, puts Chat Computer in place and leaves the rest, which needs the
# internet, to the next start: chatcomputer-finish-install.service, the same safety net a PC uses. Last, it puts
# Fedora's usual start back, deletes the sign-in copies from the EFI partition, and restarts.
#
# setup.env holds one KEY=value per line, each value base64 (so no name, password or Wi-Fi name can break it):
# USER_NAME, REAL_NAME, RECOVERY_PASSWORD, TIMEZONE, INSTALL_ID, MAC_FILES_PARTUUID, WIFI_SSID, WIFI_PSK.
set -u
ESP="${CHATCOMPUTER_ESP:-/boot/efi}"      # tests point these at a pretend Mac
T="${CHATCOMPUTER_TARGET:-}"              # the system's root: "" on the Mac
CONSOLE="${CHATCOMPUTER_CONSOLE:-/dev/console}"
P="$ESP/chatcomputer"
LOG="$P/firstboot.log"
TRIES=3

say() {
  printf '%s  %s\n' "$(date '+%H:%M:%S' 2>/dev/null)" "$*" >> "$LOG" 2>/dev/null
  printf '\nChat Computer: %s\n' "$*" > "$CONSOLE" 2>/dev/null
  return 0
}

# Gives up after a few tries, so a Mac never goes round in circles: Fedora's usual start comes back, and with
# it Fedora's own first-start questions on the screen. Nothing will use the sign-in and settings then, so
# they go too (forget_setup).
fail() {
  say "Couldn't finish setting up: $*"
  n=$(cat "$P/tries" 2>/dev/null || echo 0)
  n=$((n + 1))
  echo "$n" > "$P/tries"
  if [ "$n" -ge "$TRIES" ]; then
    say "Giving up after $n tries. The Mac now starts Fedora Linux; the details are in chatcomputer/firstboot.log on Chat Computer's EFI partition."
    put_fedora_back
    forget_setup
  else
    say "Trying again after a restart ($n of $TRIES)."
  fi
  sleep 10
  exit 1
}

put_fedora_back() {
  if [ -f "$P/fedora-grub.cfg" ]; then
    cp "$P/fedora-grub.cfg" "$ESP/EFI/fedora/grub.cfg" || return 1
  fi
  sync
}

val() {   # val KEY: the value from setup.env, decoded
  sed -n "s/^$1=//p" "$P/setup.env" | head -n 1 | base64 -d 2>/dev/null
}

# The sign-in and settings must not stay on the EFI partition (FAT32: anyone who starts the Mac can read it).
forget_setup() {
  rm -rf "$P/secrets" "$P/setup.env" "$P/settings.json" "$P/app.tar.gz"
  sync
}

command -v plymouth >/dev/null 2>&1 && plymouth quit >/dev/null 2>&1
mkdir -p "$P" 2>/dev/null
say "Setting up Chat Computer on this Mac."

# Chat Computer has no SSH server and no cockpit (linux/install-chatcomputer.sh disables both): nothing of the kind
# may listen while this first start runs, since the finish-install service needs the network and can take 20-40
# minutes, and this start is also where a kernel older than CVE-2026-98432's fix would be exposed. Fedora's Minimal
# image ships sshd and cockpit, so they are masked (a link to /dev/null) before anything else, on every try and on
# the way back to Fedora's own start.
mkdir -p "$T/etc/systemd/system" 2>/dev/null || true
for unit in sshd.service sshd.socket cockpit.service cockpit.socket; do
  ln -sf /dev/null "$T/etc/systemd/system/$unit" || fail "couldn't turn off $unit"
done

[ -f "$P/setup.env" ] || fail "its settings (chatcomputer/setup.env) are missing"
[ -f "$P/app.tar.gz" ] || fail "Chat Computer itself (chatcomputer/app.tar.gz) is missing"

USER_NAME=$(val USER_NAME)
REAL_NAME=$(val REAL_NAME)
RECOVERY=$(val RECOVERY_PASSWORD)
TIMEZONE=$(val TIMEZONE)
INSTALL_ID=$(val INSTALL_ID)
MAC_FILES=$(val MAC_FILES_PARTUUID)
WIFI_SSID=$(val WIFI_SSID)
WIFI_PSK=$(val WIFI_PSK)

case "$USER_NAME" in
  ''|[!a-z_]*|*[!a-z0-9_-]*) fail "the account name in setup.env isn't one Linux accepts" ;;
esac
case "$RECOVERY" in
  ''|*:*|*"
"*) fail "the recovery password in setup.env isn't one Linux accepts" ;;
esac
case "$INSTALL_ID" in
  ''|*[!0-9a-f]*) fail "bad install id in setup.env" ;;
esac
case "$MAC_FILES" in
  ''|*[!0-9a-fA-F-]*) MAC_FILES="" ;;
esac
case "$REAL_NAME" in *:*|*"
"*) REAL_NAME="$USER_NAME" ;; esac

echo chatcomputer > "$T/etc/hostname"

# 1. The person's account. Its password is the recovery password Chat Computer Setup showed (for repairs only:
#    Chat Computer starts without one), and it can use sudo with it (wheel), like the account on a PC.
if ! id -u "$USER_NAME" >/dev/null 2>&1; then
  useradd --create-home --comment "$REAL_NAME" --groups wheel "$USER_NAME" || fail "couldn't make the account $USER_NAME"
fi
printf '%s:%s\n' "$USER_NAME" "$RECOVERY" | chpasswd || fail "couldn't set the account's password"
RECOVERY=""
uid=$(id -u "$USER_NAME") || fail "the account $USER_NAME is missing"
gid=$(id -g "$USER_NAME") || fail "the account $USER_NAME is missing"
HOME_DIR="$T/home/$USER_NAME"
mkdir -p "$HOME_DIR"

# 2. Time zone and language.
if [ -n "$TIMEZONE" ] && [ -f "$T/usr/share/zoneinfo/$TIMEZONE" ]; then
  case "$TIMEZONE" in *..*) ;; *) ln -sf "../usr/share/zoneinfo/$TIMEZONE" "$T/etc/localtime" ;; esac
fi
[ -f "$T/etc/locale.conf" ] || echo 'LANG=en_US.UTF-8' > "$T/etc/locale.conf"

# 3. The Wi-Fi the Mac uses, for NetworkManager (root only: it holds the password).
if [ -n "$WIFI_SSID" ]; then
  case "$WIFI_SSID$WIFI_PSK" in
    *"
"*) say "The Wi-Fi name or password has a line break; skipping it (Chat Computer asks for Wi-Fi when it starts)." ;;
    *)
      d="$T/etc/NetworkManager/system-connections"
      mkdir -p "$d"
      f="$d/chatcomputer-wifi.nmconnection"
      ssid=$(printf '%s' "$WIFI_SSID" | sed 's/\\/\\\\/g')
      ( umask 077
        printf '[connection]\nid=%s\ntype=wifi\nautoconnect=true\n\n[wifi]\nmode=infrastructure\nssid=%s\n\n' "$ssid" "$ssid"
        if [ -n "$WIFI_PSK" ]; then
          printf '[wifi-security]\nkey-mgmt=wpa-psk\npsk=%s\n\n' "$(printf '%s' "$WIFI_PSK" | sed 's/\\/\\\\/g')"
        fi
        printf '[ipv4]\nmethod=auto\n\n[ipv6]\nmethod=auto\n'
      ) > "$f" || fail "couldn't save the Wi-Fi settings"
      chmod 600 "$f"
      ;;
  esac
fi

# 4. Chat Computer itself, owned by root and read-only to the person.
mkdir -p "$T/opt/chatcomputer"
rm -rf "$T/opt/chatcomputer/app.new"
mkdir -p "$T/opt/chatcomputer/app.new"
tar -xzf "$P/app.tar.gz" -C "$T/opt/chatcomputer/app.new" || fail "couldn't unpack Chat Computer"
chown -R 0:0 "$T/opt/chatcomputer/app.new"
chmod -R u=rwX,go=rX "$T/opt/chatcomputer/app.new"
rm -rf "$T/opt/chatcomputer/app"
mv "$T/opt/chatcomputer/app.new" "$T/opt/chatcomputer/app"

# 5. The person's settings, and Chat Computer's private folder, owned by them (as finish.sh does on a PC).
H="$HOME_DIR/ChatComputer"
mkdir -p "$H/secrets"
chmod 700 "$H/secrets"
if [ -f "$P/settings.json" ]; then
  cp "$P/settings.json" "$H/settings.json"
  chmod 644 "$H/settings.json"
fi
mkdir -p "$HOME_DIR/Mac"
chown -R "$uid:$gid" "$H" "$HOME_DIR/Mac"

# 6. What the rest of the install needs to know, and the safety net that finishes it on the next start.
mkdir -p "$T/etc/chatcomputer" "$T/etc/systemd/system/multi-user.target.wants"
printf '%s\n' "$USER_NAME" > "$T/etc/chatcomputer/install-user"
printf '%s\n' "$INSTALL_ID" > "$T/etc/chatcomputer/install-id"
if [ -n "$MAC_FILES" ]; then
  printf '%s\n' "$MAC_FILES" | tr 'A-F' 'a-f' > "$T/etc/chatcomputer/mac-files-partuuid"
fi
unit=chatcomputer-finish-install.service
cp "$T/opt/chatcomputer/app/linux/firstboot/$unit" "$T/etc/systemd/system/$unit" || fail "couldn't set up the rest of the install"
ln -sf "/etc/systemd/system/$unit" "$T/etc/systemd/system/multi-user.target.wants/$unit"

# 7. SELinux labels for everything written here (Fedora checks them).
if command -v restorecon >/dev/null 2>&1; then
  restorecon -RF "$T/etc/systemd/system" "$T/etc/NetworkManager/system-connections" "$T/etc/chatcomputer" \
    "$T/opt/chatcomputer" "$HOME_DIR" "$T/etc/hostname" "$T/etc/locale.conf" "$T/etc/localtime" >/dev/null 2>&1
fi

# 8. No Fedora first-start questions (Chat Computer has done what they would), Fedora's usual start from now on,
#    and no sign-in copies left on the EFI partition. Only now: if anything above failed, the Mac tries again,
#    and in the end falls back to Fedora's own first start.
rm -f "$T/etc/reconfigSys" "$T/.unconfigured"
find "$T/etc/systemd/system" -path '*.wants/initial-setup.service' -delete 2>/dev/null
put_fedora_back || fail "couldn't put Fedora's start back"
forget_setup
rm -f "$P/tries"
say "Done. Restarting to finish installing Chat Computer; that needs the internet and takes a while."
sleep 3
exit 0
