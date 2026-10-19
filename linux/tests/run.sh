#!/bin/bash
# Run the whole Chat Computer linux-base test suite.
#
# 1. shellcheck every shell script in linux/.
# 2. byte-compile / syntax-check the Python.
# 3. Run each functional test:
#      test_grub_07_chatcomputer.sh    (mocked + real grub lib, both /boot layouts)
#      test_root_helper.sh         (all verbs, PATH shims, malicious args)
#      test_session_supervisor.sh  (exit-code contract under Xvfb)
#      test_refresh_rate.sh        (each screen at its fastest refresh rate, with a stand-in xrandr)
#      test_bt_import.sh           (synthetic hive -> BlueZ info)
#      test_reclaim_installer.sh   (first-start clean-up of the installer partition, pretend disk)
#      test_install_dryrun.sh      (install-chatcomputer.sh --dry-run for Ubuntu and for Fedora)
#      test_install_chroot.sh      (real debootstrap install; opt-in)
#
# Some tests need a Python with `regipy`. We look for one and export it as
# CHATCOMPUTER_TEST_PY. If none is found we create a throwaway venv (NOT under the
# repo) and pip-install regipy there.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/.." && pwd)"

GREEN=$'\033[32m'; RED=$'\033[31m'; DIM=$'\033[2m'; RST=$'\033[0m'
total_fail=0
section() { printf '\n%s==== %s ====%s\n' "$DIM" "$1" "$RST"; }

# --- 0. Find/prepare a Python with regipy ----------------------------------
find_py_with_regipy() {
  local cand
  for cand in "${CHATCOMPUTER_TEST_PY:-}" python3 /usr/bin/python3.12 /usr/bin/python3.11; do
    [ -n "$cand" ] || continue
    command -v "$cand" >/dev/null 2>&1 || [ -x "$cand" ] || continue
    if "$cand" -c 'import regipy' >/dev/null 2>&1; then echo "$cand"; return 0; fi
  done
  return 1
}

TEST_PY="$(find_py_with_regipy || true)"
TMP_VENV=""
if [ -z "$TEST_PY" ]; then
  section "setup: creating throwaway venv with regipy (outside the repo)"
  TMP_VENV="$(mktemp -d)/venv"
  if python3 -m venv --system-site-packages "$TMP_VENV" >/dev/null 2>&1 \
     && "$TMP_VENV/bin/pip" install --quiet regipy >/dev/null 2>&1; then
    TEST_PY="$TMP_VENV/bin/python"
    echo "using $TEST_PY"
  else
    echo "WARNING: could not build a regipy venv; bt-import tests will SKIP" >&2
    TEST_PY="python3"
  fi
fi
export CHATCOMPUTER_TEST_PY="$TEST_PY"
cleanup() { [ -n "$TMP_VENV" ] && rm -rf "$(dirname "$TMP_VENV")"; }
trap cleanup EXIT

# --- 1. shellcheck ---------------------------------------------------------
section "shellcheck"
if command -v shellcheck >/dev/null 2>&1; then
  sc_fail=0
  # All shell scripts: installer, helper, session, grub, tests, and the two
  # dotfiles (which carry shellcheck shell directives).
  while IFS= read -r f; do
    if shellcheck -S warning "$f" >/tmp/sc.$$ 2>&1; then
      printf '%s ok%s   %s\n' "$GREEN" "$RST" "${f#"$REPO"/}"
    else
      printf '%s FAIL%s %s\n' "$RED" "$RST" "${f#"$REPO"/}"; cat /tmp/sc.$$; sc_fail=1
    fi
  done < <(
    printf '%s\n' \
      "$REPO/install-chatcomputer.sh" \
      "$REPO/root-helper/chatcomputer-root" \
      "$REPO/session/chatcomputer-session" \
      "$REPO/session/chatcomputer-restart" \
      "$REPO/session/chatcomputer-refresh" \
      "$REPO/session/bash_profile" \
      "$REPO/session/xinitrc" \
      "$REPO/grub/07_chatcomputer" \
      "$REPO/firstboot/finish-install.sh"
    find "$HERE" -name '*.sh'
  )
  rm -f /tmp/sc.$$
  [ "$sc_fail" -eq 0 ] && echo "shellcheck: all clean" || { echo "shellcheck: FAILURES"; total_fail=1; }
else
  echo "SKIP shellcheck not installed"
fi

# --- 2. Python syntax ------------------------------------------------------
section "python syntax"
py_fail=0
for p in "$REPO/root-helper/bt-import-windows-keys.py" "$HERE/make_system_hive.py"; do
  if "$TEST_PY" -c "import ast,sys; ast.parse(open(sys.argv[1]).read())" "$p" 2>/tmp/py.$$; then
    printf '%s ok%s   %s\n' "$GREEN" "$RST" "${p#"$REPO"/}"
  else
    printf '%s FAIL%s %s\n' "$RED" "$RST" "${p#"$REPO"/}"; cat /tmp/py.$$; py_fail=1
  fi
done
rm -f /tmp/py.$$
[ "$py_fail" -eq 0 ] || total_fail=1

# --- 3. Functional tests ---------------------------------------------------
# Decide whether to run the heavy chroot test.
if [ "${CHATCOMPUTER_RUN_CHROOT:-}" = "" ]; then
  if [ "$(id -u)" -eq 0 ] && command -v debootstrap >/dev/null 2>&1; then
    export CHATCOMPUTER_RUN_CHROOT=1
  else
    export CHATCOMPUTER_RUN_CHROOT=0
  fi
fi

run_test() {
  local name=$1
  section "$name"
  if bash "$HERE/$name"; then
    printf '%s%s: PASS%s\n' "$GREEN" "$name" "$RST"
  else
    printf '%s%s: FAIL%s\n' "$RED" "$name" "$RST"; total_fail=1
  fi
}

run_test test_grub_07_chatcomputer.sh
run_test test_root_helper.sh
run_test test_session_supervisor.sh
run_test test_refresh_rate.sh
run_test test_bt_import.sh
run_test test_finish_install.sh
run_test test_reclaim_installer.sh
run_test test_install_dryrun.sh
run_test test_install_chroot.sh

section "summary"
if [ "$total_fail" -eq 0 ]; then
  printf '%sALL TESTS PASSED%s\n' "$GREEN" "$RST"
else
  printf '%sSOME TESTS FAILED%s\n' "$RED" "$RST"
fi
exit "$total_fail"
