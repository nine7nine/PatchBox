#!/bin/sh
# Sound card hotplug, end to end: a jackdbus that sends Configure.ConstraintsChanged
# (jack2dbus-nspa, a build given by JACK2_BUILD) and the window's JACK settings page. A real card
# is "unplugged" and plugged back inside the test namespace (PB_SND_FAKE / PB_SND_UNPLUG: its
# /dev/snd links come and go); nothing touches the hardware or the desktop.
#
#   JACK2_BUILD=/path/to/jack2/build tests/nested/hotplug.sh [card=U44]
#
# Writes three screenshots of the device list (before, unplugged, back) to
# ~/.cache/patchbox-work/hotplug-*.png and checks jackdbus's log for both refreshes.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
card=${1:-U44}
jack2=${JACK2_BUILD:?set JACK2_BUILD to a jack2 build directory with dbus/jackdbus}
[ -e "/proc/asound/$card" ] || { echo "SKIP: no card $card"; exit 0; }
W=$HOME/.cache/patchbox-work
bin=$W/hotplug-bin
rm -rf "$bin"; mkdir -p "$bin"
for b in patchbox patchboxd patchbox-ctl pbx-fake-clients pbx-fake-nsm; do
  ln -s "$ROOT/target/release/$b" "$bin/$b"
done
cat > "$bin/jackdbus-nspa" <<EOF
#!/bin/sh
export LD_LIBRARY_PATH="$jack2/common" JACK_DRIVER_DIR="$jack2"
exec "$jack2/dbus/jackdbus" "\$@"
EOF
chmod +x "$bin/jackdbus-nspa"
rm -f "$W"/hotplug-*.png

script=$W/hotplug-script.txt
cat > "$script" <<'EOF'
wait 2500
click bar:settings
wait 2000
click dlg:tab:Master
wait 800
click dlg:choice:Driver
wait 500
click menu:alsa
wait 1500
click dlg:choice:ALSA+device+name
wait 600
shot HOME/.cache/patchbox-work/hotplug-1-before.png
wait 300
key escape
wait 5000
click dlg:choice:ALSA+device+name
wait 600
shot HOME/.cache/patchbox-work/hotplug-2-unplugged.png
wait 300
key escape
wait 6000
click dlg:choice:ALSA+device+name
wait 600
shot HOME/.cache/patchbox-work/hotplug-3-back.png
wait 300
key escape
wait 300
quit
EOF
sed -i "s|HOME/|$HOME/|g" "$script"

timeout 120 "$HERE/run.sh" "$W/hotplug-final.png" PB_BIN="$bin" PB_JACKDBUS=1 PB_JACKDBUS_BIN="$bin/jackdbus-nspa" \
  PB_SND_FAKE=1 PB_SND_UNPLUG="$card:8:6" "PATCHBOX_TEST=$(grep -v '^#' "$script" | tr '\n' ';')" PATCHBOX_SNAPSHOT_DELAY_MS=60000 > "$W/hotplug-run.log" 2>&1
fail=0
check() { if eval "$2"; then echo "ok   $1"; else echo "FAIL $1"; fail=1; fi; }
check "three screenshots" '[ -s "$W/hotplug-1-before.png" ] && [ -s "$W/hotplug-2-unplugged.png" ] && [ -s "$W/hotplug-3-back.png" ]'
check "the card went and came back (inside the namespace)" 'grep -q "plugged $card back" "$W/nested/unplug.log"'
n=$(grep -c "Sound cards changed" "$W/nested/jackdbus-home/.log/jack/jackdbus.log" 2>/dev/null); n=${n:-0}
check "jackdbus refreshed twice ($n)" '[ "$n" = 2 ]'
echo "screenshots: $W/hotplug-1-before.png hotplug-2-unplugged.png hotplug-3-back.png"
exit $fail
