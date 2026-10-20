#!/bin/bash
# Test session/chatcomputer-session's exit-code supervisor.
#
# We drive it with a FAKE Chat Computer command whose exit code we control, and with
# all delays time-compressed via the documented CHATCOMPUTER_* env vars. We run with
# CHATCOMPUTER_NO_X_SETUP=1 to skip xset/openbox (no real X needed for the loop
# logic), and separately smoke-test the X setup path under Xvfb if available.
#
# Assertions:
#   * exit 0  -> loop restarts after the (compressed) normal delay
#   * exit 3  -> loop restarts immediately (no crash backoff)
#   * exit !=0,3 repeated 3x within the window -> recovery screen shown
#   * ...unless the version is a new update: then it's undone (chatcomputer-root update-undo) and restarted
#   * crash backoff sequence is applied
#   * logs are written to $HOME/ChatComputer/logs/session.log and rotate
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/.." && pwd)"
SESSION="$REPO/session/chatcomputer-session"
FAILED=0
pass() { printf 'PASS %s\n' "$1"; }
fail() { printf 'FAIL %s\n' "$1"; FAILED=1; }

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
export HOME="$WORK/home"
mkdir -p "$HOME"
LOGDIR="$HOME/ChatComputer/logs"

# A fake Chat Computer: exits with the code in $WORK/exitcode, and appends a line to
# a run-counter each time it starts. Supports a "sequence" file to vary codes.
FAKE="$WORK/fake-chatcomputer"
cat > "$FAKE" <<'FK'
#!/bin/bash
runs="$FAKE_RUNS"
echo "run" >> "$runs"
n=$(wc -l < "$runs")
seq_file="$FAKE_SEQ"
if [ -f "$seq_file" ]; then
  code=$(sed -n "${n}p" "$seq_file")
  [ -n "$code" ] || code=0
else
  code=$(cat "$FAKE_EXITCODE" 2>/dev/null || echo 0)
fi
echo "fake chatcomputer run #$n exiting $code"
exit "$code"
FK
chmod +x "$FAKE"

FAKE_DOCTOR="$WORK/fake-doctor"
cat > "$FAKE_DOCTOR" <<'FD'
#!/bin/bash
echo '{"ok": true, "note": "fake doctor"}'
exit 0
FD
chmod +x "$FAKE_DOCTOR"

# A fake root helper: records its arguments, and for update-undo removes the update's marker like the real one.
# Exits with the code in $WORK/root-exit (0 if there's none).
FAKE_ROOT="$WORK/fake-root"
cat > "$FAKE_ROOT" <<FR
#!/bin/bash
echo "\$*" >> "$WORK/root-calls"
code=\$(cat "$WORK/root-exit" 2>/dev/null || echo 0)
[ "\$code" = 0 ] && rm -f "$WORK/update-pending"
echo '{"ok":true}'
exit "\$code"
FR
chmod +x "$FAKE_ROOT"

# Common env for the supervisor: time-compressed, one-shot-friendly via a hard
# run cap enforced by us (we run the session in the background and kill it once
# the expected number of runs happened).
run_supervisor() {
  # $1 = max runs to wait for before killing.
  local maxruns=$1
  : > "$WORK/runs"
  env \
    HOME="$HOME" \
    FAKE_RUNS="$WORK/runs" \
    FAKE_EXITCODE="$WORK/exitcode" \
    FAKE_SEQ="$WORK/seq" \
    CHATCOMPUTER_START_CMD="$FAKE" \
    CHATCOMPUTER_DOCTOR_CMD="$FAKE_DOCTOR" \
    CHATCOMPUTER_NO_X_SETUP=1 \
    CHATCOMPUTER_RECOVERY_NOWAIT=1 \
    CHATCOMPUTER_RESTART_DELAY=0 \
    CHATCOMPUTER_BACKOFF="0 0 0" \
    CHATCOMPUTER_CRASH_WINDOW=120 \
    CHATCOMPUTER_MAX_CRASHES=3 \
    CHATCOMPUTER_UPDATE_PENDING="$WORK/update-pending" \
    CHATCOMPUTER_ROOT_CMD="$FAKE_ROOT" \
    bash "$SESSION" > "$WORK/session.out" 2>&1 &
  local pid=$!
  # Wait until we see maxruns runs or timeout.
  local waited=0
  while [ "$(wc -l < "$WORK/runs" 2>/dev/null || echo 0)" -lt "$maxruns" ] && [ "$waited" -lt 100 ]; do
    sleep 0.1; waited=$((waited+1))
  done
  # Give it a tick more then stop.
  sleep 0.3
  kill "$pid" 2>/dev/null
  wait "$pid" 2>/dev/null
}

# --- Test 1: exit 0 restarts (normal) --------------------------------------
echo 0 > "$WORK/exitcode"; rm -f "$WORK/seq"
run_supervisor 3
runs=$(wc -l < "$WORK/runs")
if [ "$runs" -ge 3 ]; then pass "exit 0: supervisor restarts (saw $runs runs)"; else fail "exit 0: expected >=3 runs, saw $runs"; fi
if grep -q "normal exit" "$LOGDIR/session.log" 2>/dev/null; then pass "exit 0: logged 'normal exit'"; else fail "exit 0: no 'normal exit' in log"; fi

# --- Test 2: exit 3 restarts immediately -----------------------------------
echo 3 > "$WORK/exitcode"; rm -f "$WORK/seq"; : > "$LOGDIR/session.log"
run_supervisor 3
if grep -q "restart requested (exit 3)" "$LOGDIR/session.log" 2>/dev/null; then pass "exit 3: logged 'restart requested'"; else fail "exit 3: no 'restart requested' in log"; fi
runs=$(wc -l < "$WORK/runs")
if [ "$runs" -ge 3 ]; then pass "exit 3: supervisor restarts immediately (saw $runs runs)"; else fail "exit 3: expected >=3 runs, saw $runs"; fi

# --- Test 3: 3 crashes within window -> recovery ---------------------------
# Sequence: 1,1,1 (three crashes). After the 3rd, recovery is shown (NOWAIT).
printf '1\n1\n1\n0\n0\n' > "$WORK/seq"; rm -f "$WORK/exitcode"; : > "$LOGDIR/session.log"
run_supervisor 3
if grep -q "RECOVERY: entering recovery screen" "$LOGDIR/session.log" 2>/dev/null; then
  pass "3 crashes: recovery screen entered"
else
  fail "3 crashes: recovery not triggered"; sed -n '1,40p' "$LOGDIR/session.log" 2>/dev/null >&2
fi
if grep -q "fake doctor" "$WORK/session.out" 2>/dev/null || grep -q "chatcomputer doctor" "$WORK/session.out" 2>/dev/null; then
  pass "recovery: shows doctor output"
else
  fail "recovery: doctor output not shown"; sed -n '1,30p' "$WORK/session.out" >&2
fi

# --- Test 3b: a new update that keeps crashing is undone, not sent to recovery -----
printf '1\n1\n1\n0\n0\n' > "$WORK/seq"; : > "$LOGDIR/session.log"; : > "$WORK/root-calls"; rm -f "$WORK/root-exit"
date +%s > "$WORK/update-pending"
run_supervisor 4
if grep -q "going back to the version before it" "$LOGDIR/session.log" && grep -q "back on the version before the update" "$LOGDIR/session.log" \
   && [ "$(cat "$WORK/root-calls")" = "update-undo" ]; then
  pass "update that won't start: the version before is put back"
else
  fail "update that won't start: not undone"; cat "$WORK/root-calls" >&2; sed -n '1,40p' "$LOGDIR/session.log" >&2
fi
if grep -q "RECOVERY: entering recovery screen" "$LOGDIR/session.log"; then
  fail "update that won't start: recovery shown instead of going back"
else
  pass "update that won't start: no recovery screen"
fi
runs=$(wc -l < "$WORK/runs")
if [ "$runs" -ge 4 ]; then pass "update that won't start: started again ($runs runs)"; else fail "update undone: expected >=4 runs, saw $runs"; fi

# --- Test 3c: if going back fails, the recovery screen ----------------------------
printf '1\n1\n1\n0\n0\n' > "$WORK/seq"; : > "$LOGDIR/session.log"; : > "$WORK/root-calls"; echo 1 > "$WORK/root-exit"
date +%s > "$WORK/update-pending"
run_supervisor 3
if grep -q "couldn't go back to the version before the update" "$LOGDIR/session.log" \
   && grep -q "RECOVERY: entering recovery screen" "$LOGDIR/session.log"; then
  pass "update that won't start, and can't be undone: recovery screen"
else
  fail "failed undo: no recovery"; sed -n '1,40p' "$LOGDIR/session.log" >&2
fi
rm -f "$WORK/update-pending" "$WORK/root-exit"

# --- Test 4: crash backoff sequence is applied -----------------------------
# Use distinguishable non-zero backoff logging. We check the log mentions
# "restarting after backoff" for crashes below the threshold.
printf '1\n1\n0\n0\n' > "$WORK/seq"; : > "$LOGDIR/session.log"
run_supervisor 3
if grep -q "restarting after backoff" "$LOGDIR/session.log" 2>/dev/null; then pass "crash: backoff path logged"; else fail "crash: backoff path not logged"; fi
if grep -q "crash #1 within" "$LOGDIR/session.log" 2>/dev/null; then pass "crash: crash counter logged"; else fail "crash: counter not logged"; fi

# --- Test 5: log rotation --------------------------------------------------
: > "$LOGDIR/session.log"
# Force a tiny rotation threshold and generate output.
echo 0 > "$WORK/exitcode"; rm -f "$WORK/seq"
env HOME="$HOME" FAKE_RUNS="$WORK/runs" FAKE_EXITCODE="$WORK/exitcode" FAKE_SEQ="$WORK/seq" \
    CHATCOMPUTER_START_CMD="$FAKE" CHATCOMPUTER_DOCTOR_CMD="$FAKE_DOCTOR" \
    CHATCOMPUTER_NO_X_SETUP=1 CHATCOMPUTER_RECOVERY_NOWAIT=1 CHATCOMPUTER_RESTART_DELAY=0 \
    CHATCOMPUTER_BACKOFF="0 0 0" CHATCOMPUTER_LOG_MAX_BYTES=256 CHATCOMPUTER_LOG_KEEP=2 \
    bash "$SESSION" > "$WORK/session.out" 2>&1 &
pid=$!
# Up to 10 s: on a busy machine the first restarts can take longer than the second this usually takes.
for _ in $(seq 1 100); do [ -f "$LOGDIR/session.log.1" ] && break; sleep 0.1; done
kill "$pid" 2>/dev/null; wait "$pid" 2>/dev/null
if [ -f "$LOGDIR/session.log.1" ]; then pass "log rotation produced session.log.1"; else fail "log rotation did not rotate (no .1)"; ls -la "$LOGDIR" >&2; fi

# --- Test 5b: the log stays small while Chat Computer runs (for weeks), not only between runs ----------
rm -f "$LOGDIR"/session.log*
LONG="$WORK/fake-long"
cat > "$LONG" <<LF
#!/bin/bash
echo \$\$ > "$WORK/long.pid"
for i in \$(seq 1 300); do printf '%0199d\n' "\$i"; sleep 0.005; done
echo "written" > "$WORK/long.done"
sleep 30
LF
chmod +x "$LONG"
rm -f "$WORK/long.done"
env HOME="$HOME" CHATCOMPUTER_START_CMD="$LONG" CHATCOMPUTER_DOCTOR_CMD="$FAKE_DOCTOR" CHATCOMPUTER_NO_X_SETUP=1 \
    CHATCOMPUTER_LOG_MAX_BYTES=4096 CHATCOMPUTER_LOG_KEEP=2 CHATCOMPUTER_LOG_CHECK_S=0.2 \
    bash "$SESSION" > "$WORK/session.out" 2>&1 &
pid=$!
for _ in $(seq 1 150); do [ -f "$WORK/long.done" ] && break; sleep 0.1; done
sleep 0.6                                   # a look at the log after the last of it
size=$(wc -c < "$LOGDIR/session.log" 2>/dev/null || echo 0)
kept=$(cat "$LOGDIR"/session.log* 2>/dev/null | wc -c)
kill "$pid" 2>/dev/null; wait "$pid" 2>/dev/null
kill "$(cat "$WORK/long.pid" 2>/dev/null)" 2>/dev/null
if [ -f "$WORK/long.done" ] && [ "$size" -lt 4096 ] && [ -f "$LOGDIR/session.log.1" ]; then
  pass "log rotation while Chat Computer runs (session.log $size bytes, $kept kept of 60000 written)"
else
  fail "log grew while Chat Computer ran: session.log $size bytes, $kept kept"; ls -la "$LOGDIR" >&2
fi

# --- Test 6 (optional): X setup path under Xvfb ----------------------------
if command -v Xvfb >/dev/null 2>&1 && command -v xset >/dev/null 2>&1; then
  export DISPLAY=:99
  Xvfb :99 -screen 0 640x480x16 >/dev/null 2>&1 &
  XVFB=$!
  sleep 1
  # Run the session with X setup ENABLED but a fake that exits 0 immediately;
  # we only check that setup_screen's xset calls do not crash the script.
  echo 0 > "$WORK/exitcode"; rm -f "$WORK/seq"; : > "$LOGDIR/session.log"
  env HOME="$HOME" DISPLAY=:99 FAKE_RUNS="$WORK/runs" FAKE_EXITCODE="$WORK/exitcode" \
      FAKE_SEQ="$WORK/seq" CHATCOMPUTER_START_CMD="$FAKE" CHATCOMPUTER_DOCTOR_CMD="$FAKE_DOCTOR" \
      CHATCOMPUTER_RECOVERY_NOWAIT=1 CHATCOMPUTER_RESTART_DELAY=0 CHATCOMPUTER_BACKOFF="0 0 0" \
      bash "$SESSION" >/dev/null 2>&1 &
  pid=$!
  sleep 1
  kill "$pid" 2>/dev/null; wait "$pid" 2>/dev/null
  kill "$XVFB" 2>/dev/null; wait "$XVFB" 2>/dev/null
  if grep -q "session start" "$LOGDIR/session.log" 2>/dev/null; then pass "Xvfb: session ran with X setup enabled"; else fail "Xvfb: session did not log start"; fi
else
  echo "SKIP Xvfb X-setup smoke test (Xvfb/xset not installed)" >&2
fi

# --- Test 7 (optional): Ctrl+Alt+Backspace (chatcomputer-restart) ends the X server ---------------------
# The program startx waits on is chatcomputer-session, so only ending X restarts Chat Computer (xinit then ends the
# session, and the tty1 login starts it again); asking openbox to exit would leave Chat Computer running.
alive() { local s; s="$(ps -o stat= -p "$1" 2>/dev/null)"; [ -n "$s" ] && [ "${s#Z}" = "$s" ]; }
if command -v Xvfb >/dev/null 2>&1 && [ ! -e /tmp/.X98-lock ]; then
  Xvfb :98 -nolisten tcp -screen 0 640x480x16 >/dev/null 2>&1 &
  XVFB=$!
  for _ in $(seq 25); do [ -s /tmp/.X98-lock ] && break; sleep 0.2; done
  DISPLAY=:98 sh "$REPO/session/chatcomputer-restart"
  for _ in $(seq 25); do alive "$XVFB" || break; sleep 0.2; done
  if alive "$XVFB"; then
    fail "chatcomputer-restart left the X server running"; kill "$XVFB" 2>/dev/null
  else
    pass "chatcomputer-restart ends the X server of its display"
  fi
  wait "$XVFB" 2>/dev/null
else
  echo "SKIP chatcomputer-restart test (no Xvfb, or display :98 is taken)" >&2
fi

exit "$FAILED"
