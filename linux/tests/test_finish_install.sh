#!/bin/bash
# The first-start safety net (firstboot/finish-install.sh) against a fake system: stand-ins for
# systemctl, dpkg, plymouth and sleep, and an install-chatcomputer.sh that fails twice before it works.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/.." && pwd)"   # linux/
FAILED=0
pass() { printf 'PASS %s\n' "$1"; }
fail() { printf 'FAIL %s\n' "$1"; FAILED=1; }

W="$(mktemp -d)"
trap 'rm -rf "$W"' EXIT
mkdir -p "$W/bin" "$W/app/linux" "$W/etc"
for c in systemctl dpkg plymouth sleep; do
  printf '#!/bin/sh\necho "%s $*" >> "%s/calls"\n' "$c" "$W" > "$W/bin/$c"
  chmod +x "$W/bin/$c"
done
cat > "$W/app/linux/install-chatcomputer.sh" <<EOF
#!/bin/bash
n=\$(( \$(cat "$W/runs" 2>/dev/null || echo 0) + 1 ))
echo "\$n" > "$W/runs"
echo "install-chatcomputer \$*" >> "$W/calls"
[ "\$n" -ge 3 ] || exit 1
date > "$W/stamp"
EOF
echo testuser > "$W/etc/install-user"
export PATH="$W/bin:$PATH" CHATCOMPUTER_STAMP="$W/stamp" CHATCOMPUTER_INSTALL_USER_FILE="$W/etc/install-user" \
  CHATCOMPUTER_APP="$W/app" CHATCOMPUTER_RETRY_WAIT=1 CHATCOMPUTER_RETRY_WAIT_MAX=2
FI="$REPO/firstboot/finish-install.sh"

bash "$FI" > "$W/out" 2>&1
rc=$?
[ "$rc" -eq 0 ] && pass "exits 0 once the install works" || fail "exit status $rc"
[ "$(cat "$W/runs")" = 3 ] && pass "keeps trying until the install works (3 tries)" || fail "tries: $(cat "$W/runs")"
grep -q '^install-chatcomputer --user testuser --from-installer$' "$W/calls" \
  && pass "runs the installer for the right user" || fail "installer arguments"
[ "$(grep -c '^dpkg --configure -a$' "$W/calls")" = 3 ] \
  && pass "finishes half-installed packages before each try" || fail "dpkg --configure -a calls"
grep -q '^sleep 1$' "$W/calls" && grep -q '^sleep 2$' "$W/calls" \
  && pass "waits longer after each failure" || fail "waits between tries"
grep -q '^systemctl disable chatcomputer-finish-install.service$' "$W/calls" \
  && pass "turns itself off afterwards" || fail "not turned off"
tail -n 1 "$W/calls" | grep -q '^systemctl --no-block reboot$' \
  && pass "restarts the computer at the end" || fail "no restart"
grep -q "Finishing the installation" "$W/out" && pass "says what is happening" || fail "no message"

# Already installed: does nothing at all.
: > "$W/calls"
bash "$FI" > "$W/out2" 2>&1
rc=$?
{ [ "$rc" -eq 0 ] && [ ! -s "$W/calls" ]; } && pass "does nothing once installed" || fail "ran although installed"

# A bad user name in the file: refuses.
rm -f "$W/stamp"
echo 'rm -rf /' > "$W/etc/install-user"
: > "$W/calls"
bash "$FI" > "$W/out3" 2>&1
rc=$?
{ [ "$rc" -eq 1 ] && ! grep -q install-chatcomputer "$W/calls"; } && pass "refuses a bad user name" || fail "bad user name accepted"

# The unit runs the script before the login prompt and only until the base is installed.
U="$REPO/firstboot/chatcomputer-finish-install.service"
grep -q '^ExecStart=/bin/bash /opt/chatcomputer/app/linux/firstboot/finish-install.sh$' "$U" \
  && grep -q '^ConditionPathExists=!/var/lib/chatcomputer/base-installed$' "$U" \
  && grep -q '^Before=getty@tty1.service$' "$U" \
  && pass "unit: right script, runs before the login prompt, only until installed" || fail "unit file"
if command -v systemd-analyze >/dev/null 2>&1; then
  if systemd-analyze verify "$U" 2>&1 | grep -v "finish-install.sh\|Failed to prepare\|not executable" | grep -qi "error\|unknown"; then
    fail "systemd-analyze verify"; systemd-analyze verify "$U"
  else
    pass "unit passes systemd-analyze verify"
  fi
fi
exit "$FAILED"
