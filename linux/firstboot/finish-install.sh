#!/bin/bash
# Finishes installing Chat Computer on the first start when the Ubuntu installer couldn't (no internet
# at the time, a power cut, the computer turned off), and on a Mac always: there, Chat Computer's first start
# (mac/esp/firstboot.sh) leaves the whole install to this. Run by chatcomputer-finish-install.service before
# the login prompt. Keeps trying until it works, then restarts the computer into Chat Computer.
set -u
STAMP="${CHATCOMPUTER_STAMP:-/var/lib/chatcomputer/base-installed}"
USER_FILE="${CHATCOMPUTER_INSTALL_USER_FILE:-/etc/chatcomputer/install-user}"
APP="${CHATCOMPUTER_APP:-/opt/chatcomputer/app}"
WAIT="${CHATCOMPUTER_RETRY_WAIT:-30}"  # seconds before the second try; doubles up to WAIT_MAX
WAIT_MAX="${CHATCOMPUTER_RETRY_WAIT_MAX:-300}"

say() { printf '\n*** Chat Computer: %s\n\n' "$*"; }

[ -f "$STAMP" ] && exit 0
user="$(head -n 1 "$USER_FILE" 2>/dev/null || true)"
if ! [[ "$user" =~ ^[a-z_][a-z0-9_-]*$ ]]; then
  say "Can't finish installing: no valid user name in $USER_FILE."
  exit 1
fi

# Show this text rather than the boot splash.
if command -v plymouth >/dev/null 2>&1; then plymouth quit >/dev/null 2>&1 || true; fi

say "Finishing the installation. This needs the internet and can take a while. Please leave the computer on."
try=1
while :; do
  # A power cut in the middle of installing packages leaves them half set up; finish that first (Ubuntu;
  # Fedora's dnf finishes or undoes its transactions itself).
  if command -v dpkg >/dev/null 2>&1; then dpkg --configure -a >/dev/null 2>&1 || true; fi
  if bash "$APP/linux/install-chatcomputer.sh" --user "$user" --from-installer; then
    break
  fi
  say "That didn't work (try $try). Check that the internet is connected. Trying again in $WAIT seconds."
  sleep "$WAIT"
  try=$((try + 1))
  WAIT=$((WAIT * 2))
  [ "$WAIT" -gt "$WAIT_MAX" ] && WAIT="$WAIT_MAX"
done

systemctl disable chatcomputer-finish-install.service >/dev/null 2>&1 || true
say "Done. Restarting into Chat Computer."
sleep 5
systemctl --no-block reboot
