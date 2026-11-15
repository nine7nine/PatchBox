#!/bin/sh
# Sound card bridges from the settings page, end to end: a jack2dbus-nspa jackdbus (the build
# given by JACK2_BUILD) on the harness's private bus, and the Bridges tab. The snd-aloop card is
# bridged with its switch, its card period changed, then the other tabs shown; nothing plays on
# a real card and the desktop's bus and settings are never touched.
#
#   JACK2_BUILD=/path/to/jack2/build tests/nested/bridges.sh
#
# Screenshots in ~/.cache/patchbox-work/bridges-*.png; jackdbus's log is checked for the bridge
# locking, and its saved list for the new period.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
jack2=${JACK2_BUILD:?set JACK2_BUILD to a jack2 build directory with dbus/jackdbus}
[ -e /proc/asound/Loopback ] || { echo "SKIP: no snd-aloop card (modprobe snd-aloop)"; exit 0; }
W=$HOME/.cache/patchbox-work
bin=$W/bridges-bin
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
rm -f "$W"/bridges-*.png

script=$W/bridges-script.txt
cat > "$script" <<'EOF'
wait 2500
click bar:settings
wait 1500
click dlg:tab:Devices
wait 1500
shot HOME/.cache/patchbox-work/bridges-1-tab.png
click dlg:Loopback+—+a+virtual+card+(snd-aloop)
wait 9000
shot HOME/.cache/patchbox-work/bridges-2-on.png
click dlg:choice:++++++card+period
wait 600
click menu:64+frames
wait 9000
shot HOME/.cache/patchbox-work/bridges-3-period.png
click dlg:Loopback+—+a+virtual+card+(snd-aloop)
wait 2500
shot HOME/.cache/patchbox-work/bridges-3b-off-kept.png
click dlg:Loopback+—+a+virtual+card+(snd-aloop)
wait 9000
shot HOME/.cache/patchbox-work/bridges-3c-on-again.png
click dlg:tab:Server
wait 800
shot HOME/.cache/patchbox-work/bridges-4-server.png
click dlg:tab:Master
wait 800
shot HOME/.cache/patchbox-work/bridges-5-master.png
click dlg:tab:Netjack
wait 800
shot HOME/.cache/patchbox-work/bridges-6-netjack.png
quit
EOF
sed -i "s|HOME/|$HOME/|g" "$script"

timeout 150 "$HERE/run.sh" "$W/bridges-final.png" PB_BIN="$bin" PB_JACKDBUS=1 PB_JACKDBUS_BIN="$bin/jackdbus-nspa" \
  "PATCHBOX_TEST=$(grep -v '^#' "$script" | tr '\n' ';')" PATCHBOX_SNAPSHOT_DELAY_MS=90000 > "$W/bridges-run.log" 2>&1
fail=0
check() { if eval "$2"; then echo "ok   $1"; else echo "FAIL $1"; fail=1; fi; }
log=$W/nested/jackdbus-home/.log/jack/jackdbus.log
conf=$W/nested/jackdbus-home/.config/jack/conf.xml
check "eight screenshots" 'for i in 1-tab 2-on 3-period 3b-off-kept 3c-on-again 4-server 5-master 6-netjack; do [ -s "$W/bridges-$i.png" ] || exit 1; done'
check "the switch bridged the card, and it locked" 'grep -q "bridge Loopback: locked" "$log"'
check "the new period: the bridge again, locked" '[ "$(grep -c "bridge Loopback: locked" "$log")" -ge 2 ] && grep -q "period 64 x 2" "$log"'
check "switched off and on again: its period kept (Loopback/64 saved)" '[ "$(grep -c "period 64 x 2" "$log")" -ge 2 ] && grep -q "<option name=\"bridges\">Loopback/64</option>" "$conf"'
grep "bridge" "$log" | cut -c26- | sed 's/^/    /'
echo "screenshots: $W/bridges-*.png"
exit $fail
