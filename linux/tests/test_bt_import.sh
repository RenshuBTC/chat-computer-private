#!/bin/bash
# Test the Bluetooth link-key importer end-to-end.
#
# Builds a SYNTHETIC Windows SYSTEM hive (no external tooling, pure Python via
# tests/make_system_hive.py) containing BTHPORT classic link keys, runs the real
# importer (root-helper/bt-import-windows-keys.py) against it, and asserts the
# resulting BlueZ info files. Also unit-tests the pure formatting functions, and
# checks that existing [General] sections are preserved and that re-running is
# idempotent.
#
# Requires regipy in the test Python. CHATCOMPUTER_TEST_PY must point at a venv/py
# with regipy installed (run.sh arranges this).
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/.." && pwd)"
IMPORTER="$REPO/root-helper/bt-import-windows-keys.py"
MAKER="$HERE/make_system_hive.py"
PY="${CHATCOMPUTER_TEST_PY:-python3}"
FAILED=0
pass() { printf 'PASS %s\n' "$1"; }
fail() { printf 'FAIL %s\n' "$1"; FAILED=1; }

# Guard: need regipy.
if ! "$PY" -c 'import regipy' 2>/dev/null; then
  echo "SKIP bt-import tests: regipy not available in \$CHATCOMPUTER_TEST_PY ($PY)" >&2
  exit 0
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
HIVE="$WORK/SYSTEM"
BT="$WORK/bluetooth"
ADAPTER="0A:1B:2C:3D:4E:5F"
DEV1="A1:B2:C3:D4:E5:F6"
DEV2="C0:FF:EE:12:34:56"
KEY1="0123456789ABCDEF0123456789ABCDEF"
KEY2="FEDCBA9876543210FEDCBA9876543210"

# Build the synthetic hive.
if "$PY" "$MAKER" "$HIVE" >/dev/null 2>&1; then pass "built synthetic SYSTEM hive"; else fail "could not build synthetic hive"; exit "$FAILED"; fi

# Pre-create the local adapter dir so the importer writes into it.
mkdir -p "$BT/$ADAPTER"

# --- Unit-test the pure functions ------------------------------------------
"$PY" - "$IMPORTER" <<'PY'
import importlib.util, sys
spec = importlib.util.spec_from_file_location("bt", sys.argv[1])
m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m)
ok = True
def check(cond, msg):
    global ok
    print(("PASS " if cond else "FAIL ") + msg)
    ok = ok and cond
# mac_from_hex
check(m.mac_from_hex("0a1b2c3d4e5f") == "0A:1B:2C:3D:4E:5F", "mac_from_hex basic")
check(m.mac_from_hex("A1B2C3D4E5F6") == "A1:B2:C3:D4:E5:F6", "mac_from_hex uppercases")
# key_to_bluez_hex: byte order preserved, uppercase
kb = bytes.fromhex("fedcba9876543210fedcba9876543210")
check(m.key_to_bluez_hex(kb) == "FEDCBA9876543210FEDCBA9876543210", "key_to_bluez_hex no reversal + upper")
# extract_value_bytes handles bytes, hex-string, and list forms
class V:  # minimal stand-in for a regipy value
    def __init__(self, value): self.value = value
check(m.extract_value_bytes(V(b"\x01\x02")) == b"\x01\x02", "extract bytes")
check(m.extract_value_bytes(V("0102")) == b"\x01\x02", "extract hex string")
check(m.extract_value_bytes(V([1, 2, 255])) == bytes([1,2,255]), "extract int list")
# name_from_value: what Windows calls a device (Devices\<mac>\Name), as regipy hands it over
class N:
    def __init__(self, value, value_type): self.value, self.value_type = value, value_type
airpods = "Sam\u2019s AirPods Pro"
check(m.name_from_value(N((airpods.encode() + b"\x00junk").hex(), "REG_BINARY")) == airpods, "name from REG_BINARY hex")
check(m.name_from_value(N(airpods.encode() + b"\x00", "REG_BINARY")) == airpods, "name from REG_BINARY bytes")
check(m.name_from_value(N(airpods, "REG_SZ")) == airpods, "name from REG_SZ")
check(m.name_from_value(N("Bad\nName\x07", "REG_SZ")) == "BadName", "name keeps printable characters only")
check(len(m.name_from_value(N(("x" * 300).encode().hex(), "REG_BINARY")).encode()) == 248, "name at most 248 bytes")
check(m.name_from_value(N(None, "REG_NONE")) == "", "no name")
sys.exit(0 if ok else 1)
PY
[ $? -eq 0 ] && pass "pure-function unit tests" || fail "pure-function unit tests"

# --- End-to-end import -----------------------------------------------------
OUT="$("$PY" -I "$IMPORTER" --hive "$HIVE" --bluetooth-dir "$BT" 2>/dev/null)"
echo "$OUT" | "$PY" -m json.tool >/dev/null 2>&1 && pass "importer emits valid JSON" || { fail "importer JSON invalid: $OUT"; }

cs="$(echo "$OUT" | "$PY" -c 'import sys,json;print(json.load(sys.stdin)["controlset"])')"
[ "$cs" = '\ControlSet001' ] && pass "resolved ControlSet001 via Select\\Current" || fail "wrong control set: $cs"

count="$(echo "$OUT" | "$PY" -c 'import sys,json;print(json.load(sys.stdin)["count"])')"
[ "$count" = "2" ] && pass "imported exactly 2 classic keys (skipped non-16-byte)" || fail "expected 2 keys, got $count"
changed="$(echo "$OUT" | "$PY" -c 'import sys,json;print(json.load(sys.stdin)["changed"])')"
[ "$changed" = "2" ] && pass "reports 2 pairings changed (so bluetoothd is restarted)" || fail "expected changed=2, got $changed"
named="$(echo "$OUT" | "$PY" -c 'import sys,json;print([i.get("name","") for i in json.load(sys.stdin)["imported"]])')"
[ "$named" = "['Sam’s AirPods Pro', '']" ] && pass "reports what Windows calls each device" || fail "names reported: $named"

# info file 1
F1="$BT/$ADAPTER/$DEV1/info"
if [ -f "$F1" ]; then
  pass "wrote info for $DEV1"
  grep -q "^\[LinkKey\]" "$F1" && pass "$DEV1: has [LinkKey] section" || fail "$DEV1: missing [LinkKey]"
  grep -q "^Key=$KEY1$" "$F1" && pass "$DEV1: Key correct (uppercase, same byte order)" || { fail "$DEV1: wrong Key"; cat "$F1"; }
  grep -q "^Type=4$" "$F1" && pass "$DEV1: Type=4" || fail "$DEV1: Type!=4"
  grep -q "^PINLength=0$" "$F1" && pass "$DEV1: PINLength=0" || fail "$DEV1: PINLength!=0"
  # Windows' name for it, so Chat Computer knows them for AirPods, and trusted so they connect by themselves.
  grep -q "^Name=Sam’s AirPods Pro$" "$F1" && pass "$DEV1: Name from Windows" || { fail "$DEV1: no Name"; cat "$F1"; }
  grep -q "^Trusted=true$" "$F1" && grep -q "^Blocked=false$" "$F1" && grep -q "^SupportedTechnologies=BR/EDR;$" "$F1" \
    && pass "$DEV1: trusted, not blocked, classic Bluetooth" || { fail "$DEV1: [General] incomplete"; cat "$F1"; }
  # The link key is readable by root only, as bluetoothd keeps it.
  [ "$(stat -c %a "$F1")" = 600 ] && [ "$(stat -c %a "$(dirname "$F1")")" = 700 ] \
    && pass "$DEV1: info file 0600 in a 0700 folder" || fail "$DEV1: modes $(stat -c %a "$F1") / $(stat -c %a "$(dirname "$F1")")"
else
  fail "no info file for $DEV1"
fi

# info file 2 (matches bt-dualboot's real fixture key)
F2="$BT/$ADAPTER/$DEV2/info"
grep -q "^Key=$KEY2$" "$F2" 2>/dev/null && pass "$DEV2: Key matches known-good value" || fail "$DEV2: Key wrong"
# Windows has no name for this one: none is made up (BlueZ learns it when the device connects).
! grep -q "^Name=" "$F2" && grep -q "^Trusted=true$" "$F2" && pass "$DEV2: no made-up name, still trusted" \
  || { fail "$DEV2: [General] wrong"; cat "$F2"; }

# MasterIRK (17 bytes) must NOT have produced a device dir.
shopt -s nullglob
irk_hits=("$BT/$ADAPTER"/*[Mm][Aa][Ss][Tt][Ee][Rr]* "$BT/$ADAPTER"/00:11:22*)
shopt -u nullglob
if [ "${#irk_hits[@]}" -gt 0 ]; then fail "MasterIRK was imported (should be skipped)"; else pass "non-classic MasterIRK skipped"; fi

# --- Preserve other sections -----------------------------------------------
# Put a [General] section in an existing info file; import must keep it.
mkdir -p "$BT/$ADAPTER/$DEV1"
cat > "$F1" <<EOF
[General]
Name=Test AirPods
Trusted=true

[LinkKey]
Key=DEADBEEFDEADBEEFDEADBEEFDEADBEEF
Type=4
PINLength=0
EOF
"$PY" -I "$IMPORTER" --hive "$HIVE" --bluetooth-dir "$BT" >/dev/null 2>&1
if grep -q "^Name=Test AirPods$" "$F1" && grep -q "^Trusted=true$" "$F1"; then
  pass "import preserves existing [General] section"
else
  fail "import clobbered [General] section"; cat "$F1"
fi
if grep -q "^Key=$KEY1$" "$F1"; then pass "import refreshed the LinkKey (DEADBEEF -> real key)"; else fail "LinkKey not refreshed"; cat "$F1"; fi
[ "$(stat -c %a "$F1")" = 600 ] && pass "a refreshed info file is 0600 too" || fail "refreshed info file mode $(stat -c %a "$F1")"

# --- Idempotency -----------------------------------------------------------
"$PY" -I "$IMPORTER" --hive "$HIVE" --bluetooth-dir "$BT" >/dev/null 2>&1
sum1="$(cat "$F1" "$F2" | sha256sum)"
O2="$("$PY" -I "$IMPORTER" --hive "$HIVE" --bluetooth-dir "$BT" 2>/dev/null)"
sum2="$(cat "$F1" "$F2" | sha256sum)"
[ "$sum1" = "$sum2" ] && pass "importer is idempotent (info files unchanged on re-run)" || fail "importer not idempotent"
# Second run should report 'unchanged' actions, and nothing changed (bluetoothd isn't restarted for nothing).
echo "$O2" | grep -q 'unchanged' && pass "re-run reports 'unchanged' actions" || echo "NOTE: re-run actions: $(echo "$O2" | "$PY" -c 'import sys,json;print([i["action"] for i in json.load(sys.stdin)["imported"]])' 2>/dev/null)" >&2
c2="$(echo "$O2" | "$PY" -c 'import sys,json;print(json.load(sys.stdin)["changed"])')"
[ "$c2" = "0" ] && pass "re-run reports nothing changed" || fail "re-run changed=$c2"

exit "$FAILED"
