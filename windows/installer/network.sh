#!/bin/sh
# Chat Computer: name the Wi-Fi card for the installer. Runs in the Ubuntu installer as an early command;
# the installer re-reads /autoinstall.yaml afterwards.
#
# Setup on Windows can't know what Linux will call the Wi-Fi card, so it writes the Wi-Fi network
# under "any-wifi" with "match: {name: wl*}". The installer's network backend (systemd-networkd) only
# accepts Wi-Fi by interface name, so this puts the real name in (e.g. wlp2s0), or removes the Wi-Fi
# part when the PC has no Wi-Fi (a cable is used instead).
set -u
CFG="${CHATCOMPUTER_AUTOINSTALL:-/autoinstall.yaml}"
NET="${CHATCOMPUTER_NET_SYS:-/sys/class/net}"
[ -f "$CFG" ] || exit 0
grep -q '^    wifis:$' "$CFG" || exit 0

find_wifi() {
  for d in "$NET"/*; do
    [ -e "$d" ] || continue
    if [ -d "$d/wireless" ] || [ -e "$d/phy80211" ]; then basename "$d"; return 0; fi
  done
  return 1
}

# Wi-Fi drivers can take a few seconds to appear.
iface=""
tries=0
while [ "$tries" -lt "${CHATCOMPUTER_NET_WAIT:-15}" ]; do
  iface=$(find_wifi) && break
  iface=""
  tries=$((tries + 1))
  sleep 1
done

case "$iface" in
  *[!A-Za-z0-9_.-]*) iface="" ;;
esac

tmp="$CFG.chatcomputer.tmp"
awk -v iface="$iface" '
  /^    wifis:$/ { if (iface == "") { skipping = 1; next } }
  skipping == 1 { if ($0 ~ /^      /) next; skipping = 0 }
  iface != "" && $0 == "      any-wifi:" { print "      " iface ":"; inwifi = 1; next }
  inwifi == 1 && $0 ~ /^        match: \{name: "wl\*"\}$/ { next }
  { print }
' "$CFG" > "$tmp" && mv "$tmp" "$CFG"

if [ -n "$iface" ]; then
  echo "chatcomputer-network: Wi-Fi card is $iface"
  # The installer normally adds Wi-Fi support itself when it sees the card; make sure it's there.
  if ! command -v wpa_supplicant >/dev/null 2>&1; then
    DEBIAN_FRONTEND=noninteractive apt-get install -y wpasupplicant >/dev/null 2>&1 || true
  fi
else
  echo "chatcomputer-network: no Wi-Fi card found; using a network cable"
fi
exit 0
