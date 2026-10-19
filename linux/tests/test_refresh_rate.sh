#!/bin/bash
# chatcomputer-refresh: each connected screen goes to the fastest refresh rate xrandr lists for the size it's at
# (X starts most fast screens at 60 Hz, where Windows has them at 120 or 144 Hz, and the mouse's arrow feels slow).
# xrandr is a stand-in that prints a real-looking listing and records what it's asked to do.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
SCRIPT="$HERE/../session/chatcomputer-refresh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
fail=0
t() { local name=$1; shift; if "$@"; then echo "ok   $name"; else echo "FAIL $name"; fail=1; fi; }
has() { [[ "$1" == *"$2"* ]]; }
lacks() { [[ "$1" != *"$2"* ]]; }
none() { [ ! -e "$TMP/asked" ]; }

mkdir -p "$TMP/bin" "$TMP/home"
cat > "$TMP/bin/xrandr" <<'EOF'
#!/bin/bash
if [ "$1" = "--query" ]; then
  cat "$LISTING"
  exit 0
fi
echo "$*" >> "$ASKED"
[ "$2" = "${REFUSE:-none}" ] && exit 1
exit 0
EOF
chmod +x "$TMP/bin/xrandr"
cat > "$TMP/listing" <<'EOF'
Screen 0: minimum 320 x 200, current 6400 x 1440, maximum 16384 x 16384
DP-1 connected primary 2560x1440+0+0 (normal left inverted right x axis y axis) 597mm x 336mm
   2560x1440     59.95*+ 143.97   119.88    99.95
   2560x1440     165.00
   1920x1080    240.00   144.00    60.00
   1920x1080i    60.00
HDMI-1 connected 1920x1080+2560+0 (normal left inverted right x axis y axis) 527mm x 296mm
   1920x1080     60.00*+  50.00    59.94
   1280x720      60.00
eDP-1 connected 1920x1200+4480+0 (normal left inverted right x axis y axis) 300mm x 190mm
   1920x1200     60.00 +  48.00*
DP-2 disconnected (normal left inverted right x axis y axis)
   3840x2160     60.00
EOF

run() { LISTING="$TMP/listing" ASKED="$TMP/asked" PATH="$TMP/bin:$PATH" HOME="$TMP/home" bash "$SCRIPT"; }

out=$(REFUSE=eDP-1 run)
asked=$(cat "$TMP/asked")
# The fastest rate at the size it's at, from any line for that size (not 240 Hz at a smaller size).
t "a 60 Hz screen goes to its fastest rate at its size" has "$asked" "--output DP-1 --mode 2560x1440 --rate 165.00"
t "a screen already at its fastest is left alone" lacks "$asked" HDMI-1
t "a disconnected screen is left alone" lacks "$asked" DP-2
t "a slower current rate goes up too" has "$asked" "--output eDP-1 --mode 1920x1200 --rate 60.00"
t "the log says what changed" has "$out" "screen DP-1: 2560x1440 at 165.00 Hz (was 59.95 Hz)"
t "and what couldn't" has "$out" "screen eDP-1: 1920x1200 stays at 48.00 Hz (60.00 Hz didn't work)"

# Already at their fastest: nothing is asked.
rm -f "$TMP/asked"
printf '%s\n' "Screen 0: minimum 8 x 8" "HDMI-1 connected 1920x1080+0+0 (normal) 527mm x 296mm" \
  "   1920x1080     60.00*+  50.00    59.94" > "$TMP/listing"
out=$(run)
t "nothing to change: nothing asked" none
t "nothing to change: said so" has "$out" "already at its fastest"

# Kept as they are when asked to.
printf '%s\n' "Screen 0: minimum 8 x 8" "DP-1 connected 2560x1440+0+0 (normal) 597mm x 336mm" \
  "   2560x1440     59.95*+ 143.97" > "$TMP/listing"
mkdir -p "$TMP/home/ChatComputer" && touch "$TMP/home/ChatComputer/keep-refresh-rate"
out=$(run)
t "keep-refresh-rate leaves the rates alone" none
t "and says so" has "$out" "left as they are"
rm -f "$TMP/home/ChatComputer/keep-refresh-rate"
CHATCOMPUTER_KEEP_REFRESH=1 run >/dev/null
t "CHATCOMPUTER_KEEP_REFRESH=1 too" none

# Without xrandr: nothing happens, and that's fine.
mkdir -p "$TMP/empty"
out=$(PATH="$TMP/empty" /bin/bash "$SCRIPT"; echo "rc=$?")
t "no xrandr: quietly nothing" [ "$out" = "rc=0" ]

exit "$fail"
