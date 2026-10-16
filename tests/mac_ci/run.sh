#!/bin/bash
# The Mac test: ClaudeOS on a real Mac, start to finish. GitHub's Macs run it (.github/workflows/mac.yml); it
# also runs on any Mac where installing and then removing ClaudeOS is fine (it uses the real ~/ClaudeOS and
# ~/Applications, and removes them at the end).
#
#   bash tests/mac_ci/run.sh OUTDIR
#
#   1. setup: mac/setup.sh, answered the way a person would (a stand-in Claude sign-in, since nobody can sign
#      in here; everything else is real: uv, Python, the packages, the voice download, the app, the login item)
#   2. what setup made: the app's signature and architectures, launch.conf, "python -m claudeos doctor"
#   3. the app, opened the way macOS opens it: does it run, fill the screen, and quit when asked
#   4. ClaudeOS on the screen with a scripted Claude: talking, websites, documents, sleep, "switch to macOS"
#      and back, quitting (tests/mac_ci/mac_scenario.py)
#   5. the Python tests, on the Mac
#   6. setup's "Remove ClaudeOS from this Mac"
# Everything it sees goes to OUTDIR: summary.txt (PASS/FAIL per check), logs and screenshots. Exit status 1
# if any check failed. Written for macOS's own bash 3.2.
set -u
REPO="$(cd "$(dirname "$0")/../.." && pwd)"
OUT="${1:?usage: run.sh OUTDIR}"
mkdir -p "$OUT"
OUT="$(cd "$OUT" && pwd)"
CO="$HOME/ClaudeOS"
APP="$HOME/Applications/ClaudeOS.app"
AGENT="$HOME/Library/LaunchAgents/com.claudeos.start.plist"
APP_PROC="ClaudeOS.app/Contents/MacOS/ClaudeOS"
FAILED=0
: > "$OUT/summary.txt"

pass() { printf 'PASS  %s\n' "$*" | tee -a "$OUT/summary.txt"; }
fail() { printf 'FAIL  %s\n' "$*" | tee -a "$OUT/summary.txt"; FAILED=1; }
note() { printf '      %s\n' "$*" | tee -a "$OUT/summary.txt"; }
check() {   # check WHAT COMMAND...: PASS if the command succeeds (its output goes to checks.log)
  local what="$1"
  shift
  printf '\n$ %s\n' "$*" >> "$OUT/checks.log"
  if "$@" >> "$OUT/checks.log" 2>&1; then pass "$what"; else fail "$what"; fi
}
with_timeout() {   # with_timeout SECONDS COMMAND...: macOS has no timeout(1); an alarm survives exec
  local secs="$1"
  shift
  perl -e 'alarm shift @ARGV; exec @ARGV or exit 127' "$secs" "$@"
}
app_running() { pgrep -f "$APP_PROC" >/dev/null 2>&1; }
section() { printf '\n== %s\n' "$*" | tee -a "$OUT/summary.txt"; }

# ------------------------------------------------------------------ 0. this Mac
section "This Mac"
{
  sw_vers
  uname -a
  sysctl -n machdep.cpu.brand_string 2>/dev/null
  printf 'arm64 hardware: %s\n' "$(sysctl -n hw.optional.arm64 2>/dev/null || echo 0)"
  printf 'bash: %s\n' "$BASH_VERSION"
  df -h "$HOME"
  system_profiler SPDisplaysDataType SPAudioDataType 2>/dev/null
} > "$OUT/0-this-mac.txt" 2>&1
note "macOS $(sw_vers -productVersion) on $(uname -m)"

# ------------------------------------------------------------------ 1. setup, as a person runs it
section "1. Setup (mac/setup.sh)"
# Nobody can sign in to Claude here: a stand-in sign-in makes setup skip that step ("already signed in").
mkdir -p "$CO/secrets"
chmod 700 "$CO/secrets"
(umask 077 && printf 'sk-ant-oat01-%s\n' "$(head -c 90 /dev/zero | tr '\0' 'x')" > "$CO/secrets/claude_token")
# Answers: Install; Go ahead; don't sign in again; open at login: yes; start now: no (step 3 opens it).
t0=$(date +%s)
printf '1\ny\nn\ny\nn\n' | with_timeout 1800 bash "$REPO/mac/setup.sh" > "$OUT/1-setup.txt" 2>&1
rc=$?
note "setup took $(( $(date +%s) - t0 )) seconds"
cp "$CO/logs/setup.log" "$OUT/1-setup.log" 2>/dev/null
if [ $rc = 0 ]; then pass "setup installs ClaudeOS"; else fail "setup installs ClaudeOS (exit $rc; see 1-setup.txt)"; fi

# ------------------------------------------------------------------ 2. what setup made
section "2. What setup installed"
check "the app is in Applications" test -x "$APP/Contents/MacOS/ClaudeOS"
check "its signature is intact" codesign --verify --deep --strict --verbose=2 "$APP"
check "it's one program for Apple silicon and Intel" sh -c "lipo -archs '$APP/Contents/MacOS/ClaudeOS' | grep -q arm64 && lipo -archs '$APP/Contents/MacOS/ClaudeOS' | grep -q x86_64"
check "no quarantine mark on it" sh -c "! xattr -lr '$APP' | grep -q com.apple.quarantine"
{
  codesign -dvvv "$APP"
  lipo -archs "$APP/Contents/MacOS/ClaudeOS"
  printf -- '--- Gatekeeper (an ad hoc signed app is expected to be "rejected" here; it still opens):\n'
  spctl --assess --type execute -vv "$APP"
  plutil -p "$APP/Contents/Info.plist"
  cat "$CO/launch.conf"
} > "$OUT/2-app.txt" 2>&1
PY="$(sed -n 's/^python=//p' "$CO/launch.conf" 2>/dev/null)"
LIBPY="$(sed -n 's/^libpython=//p' "$CO/launch.conf" 2>/dev/null)"
check "launch.conf names ClaudeOS's Python and its library" sh -c "test -x '$PY' && test -f '$LIBPY'"
check "the login item is a valid LaunchAgent" plutil -lint "$AGENT"
check "the voice and speech recognition downloaded" sh -c "test -f '$CO/models/kokoro/kokoro-v1.0.int8.onnx' && test -f '$CO/models/whisper/faster-whisper-small.en/model.bin'"
with_timeout 180 "$CO/runtime/bin/python" -m claudeos doctor > "$OUT/2-doctor.json" 2> "$OUT/2-doctor.err"
rc=$?
if [ $rc = 0 ]; then pass "\"claudeos doctor\" says it's usable"; else fail "\"claudeos doctor\" says it's usable (exit $rc; see 2-doctor.json)"; fi

# ------------------------------------------------------------------ 3. the app, as macOS opens it
section "3. The app"
open "$APP" > "$OUT/3-open.txt" 2>&1
for _ in $(seq 1 60); do app_running && break; sleep 1; done
sleep 30
screencapture -x "$OUT/3-app.png" 2>> "$OUT/3-open.txt"
pgrep -fl "$APP_PROC" > "$OUT/3-processes.txt" 2>&1
if app_running; then pass "it opens and keeps running"; else fail "it opens and keeps running"; fi
# Its windows, as the window server lists them.
"$CO/runtime/bin/python" - > "$OUT/3-windows.json" 2>&1 <<'EOF'
import json
import Quartz
from AppKit import NSScreen
wins = Quartz.CGWindowListCopyWindowInfo(Quartz.kCGWindowListOptionOnScreenOnly, Quartz.kCGNullWindowID) or []
f = NSScreen.mainScreen().frame()
out = {"screen": [f.size.width, f.size.height], "windows": []}
for w in wins:
    b = w.get("kCGWindowBounds") or {}
    out["windows"].append({"owner": w.get("kCGWindowOwnerName"), "name": w.get("kCGWindowName"),
                           "layer": w.get("kCGWindowLayer"), "w": b.get("Width"), "h": b.get("Height")})
print(json.dumps(out, indent=1, default=str))
EOF
check "its window fills the screen" "$CO/runtime/bin/python" -c '
import json, sys
d = json.load(open(sys.argv[1]))
sw, sh = d["screen"]
mine = [w for w in d["windows"] if w["owner"] == "ClaudeOS"]
print(mine)
sys.exit(0 if any(w["w"] == sw and w["h"] == sh for w in mine) else 1)' "$OUT/3-windows.json"
cp "$CO/logs/claudeos.log" "$OUT/3-claudeos.log" 2>/dev/null
# Quitting: the Apple event Command-Q also sends; ClaudeOS stops cleanly, then macOS ends it.
t0=$(date +%s)
with_timeout 30 osascript -e 'tell application id "com.claudeos.app" to quit' > "$OUT/3-quit.txt" 2>&1
for _ in $(seq 1 20); do app_running || break; sleep 1; done
if app_running; then
  fail "it quits when asked (still running after $(( $(date +%s) - t0 )) seconds)"
  pkill -9 -f "$APP_PROC"
else
  pass "it quits when asked ($(( $(date +%s) - t0 )) seconds)"
fi
cp "$CO/logs/claudeos.log" "$OUT/3-claudeos.log" 2>/dev/null
log show --style compact --last 10m --predicate 'process == "tccd"' 2>/dev/null | grep -i -E "claudeos|python" | tail -80 > "$OUT/3-permissions.txt"
# The microphone question ClaudeOS asked stays on the screen (nobody here can answer it); close it.
killall -q UserNotificationCenter universalAccessAuthWarn 2>/dev/null
cp "$HOME"/Library/Logs/DiagnosticReports/*ClaudeOS* "$HOME"/Library/Logs/DiagnosticReports/*Python* "$OUT/" 2>/dev/null

# ------------------------------------------------------------------ 4. ClaudeOS on the screen, scripted Claude
section "4. Talking to it (scripted Claude)"
UV="$(ls "$CO"/.setup/uv-*.data/scripts/uv 2>/dev/null | head -n 1)"
if [ -z "$UV" ]; then
  python3 -m pip install --quiet --user uv > "$OUT/4-test-packages.txt" 2>&1
  UV="$(python3 -c 'import sysconfig; print(sysconfig.get_path("scripts", "posix_user"))')/uv"
fi
# Test-only packages, into ClaudeOS's Python: the pretend Claude (aiohttp) and the test runner.
"$UV" pip install --python "$CO/runtime/bin/python" aiohttp pytest pyyaml jsonschema >> "$OUT/4-test-packages.txt" 2>&1
t0=$(date +%s)
# Inside the real app (its program started directly, so the test can hand it the scenario): ClaudeOS as macOS
# sees it, with its name and permissions, and coming to the front the way the app does.
mkdir -p "$OUT/scenario"
CLAUDEOS_TEST_MODELS="$CO/models" CLAUDEOS_TEST_SCENARIO="$REPO/tests/mac_ci/mac_scenario.py" \
  CLAUDEOS_TEST_SCENARIO_ARGS="$OUT/scenario" with_timeout 900 "$APP/Contents/MacOS/ClaudeOS" > "$OUT/4-scenario.txt" 2>&1
rc=$?
cp "$CO/logs/app.log" "$OUT/4-app.log" 2>/dev/null
"$CO/runtime/bin/python" - "$OUT/scenario/result.json" "$rc" "$(( $(date +%s) - t0 ))" 2>&1 <<'EOF' | tee -a "$OUT/summary.txt"
import json, sys, time
path, rc, took = sys.argv[1], int(sys.argv[2]), sys.argv[3]
try:
    r = json.load(open(path))
except Exception as exc:
    print(f"FAIL  the scenario ran (no result: {exc}; exit {rc})")
    sys.exit()
for s in r.get("steps", []):
    print(("PASS  " if s["ok"] else "FAIL  ") + s["step"])
if r.get("error"):
    print("FAIL  the scenario finished (it stopped with an error; see scenario/result.json)")
quit_s = time.time() - r.get("quit_asked_at", time.time())
print(("PASS  " if rc == 0 else "FAIL  ") + f"it quits cleanly, the way Command-Q does (exit {rc})")
print(f"      voice models used: {r.get('voice_models')}; {r.get('spoken_chunks', 0)} spoken pieces; took {took} s")
EOF
grep -q '^FAIL' "$OUT/summary.txt" && FAILED=1
cp "$OUT/scenario/home/ClaudeOS/logs/claudeos.log" "$OUT/4-claudeos.log" 2>/dev/null

# ------------------------------------------------------------------ 5. the Python tests
section "5. The Python tests"
# The voice tests find the models in one folder (tests/test_voice.py): the ones setup downloaded.
TM="$(mktemp -d)"
ln -s "$CO/models/kokoro/kokoro-v1.0.int8.onnx" "$CO/models/kokoro/voices-v1.0.bin" "$TM/"
ln -s "$CO/models/whisper" "$TM/whisper"
(cd "$REPO" && CLAUDEOS_TEST_MODELS="$TM" with_timeout 1200 "$CO/runtime/bin/python" -m pytest -q -p no:cacheprovider -rs tests) \
  > "$OUT/5-pytest.txt" 2>&1
rc=$?
note "$(tail -n 1 "$OUT/5-pytest.txt")"
if [ $rc = 0 ]; then pass "the Python tests pass on this Mac"; else fail "the Python tests pass on this Mac (see 5-pytest.txt)"; fi

# ------------------------------------------------------------------ 6. removing it
section "6. Remove (setup's fourth choice)"
# Answers: Remove; type REMOVE; don't keep what it remembers.
printf '4\nREMOVE\nn\n' | with_timeout 300 bash "$REPO/mac/setup.sh" > "$OUT/6-remove.txt" 2>&1
rc=$?
if [ $rc = 0 ]; then pass "setup removes ClaudeOS"; else fail "setup removes ClaudeOS (exit $rc)"; fi
check "the ClaudeOS folder is gone" test ! -e "$CO"
check "the app is gone" test ! -e "$APP"
check "the login item is gone" test ! -e "$AGENT"
check "ClaudeOS isn't running" sh -c "! pgrep -f '$APP_PROC'"

section "Done"
grep -c '^PASS' "$OUT/summary.txt" | sed 's/^/      passed: /' | tee -a "$OUT/summary.txt"
grep -c '^FAIL' "$OUT/summary.txt" | sed 's/^/      failed: /' | tee -a "$OUT/summary.txt"
exit "$FAILED"
