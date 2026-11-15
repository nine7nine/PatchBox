#!/bin/sh
# The toolbar's DMA LATENCY tile, end to end: a JackBus jackdbus (the build given by
# JACK2_BUILD) on the harness's private bus. Its menu offers levels from this machine's idle
# states (sysfs cpuidle); the first real one (the lowest latency that lets CPUs idle) is picked,
# then off again. Only jackdbus's cpu-dma-latency is set: while the test server runs it holds
# that PM QoS request (if it may open /dev/cpu_dma_latency), and releases it when set off.
#
#   JACK2_BUILD=/path/to/jack2/build tests/nested/cpuwake.sh
#
# Screenshots in ~/.cache/patchbox-work/cpuwake-*.png; jackdbus's saved settings and its log are
# checked.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
jack2=${JACK2_BUILD:?set JACK2_BUILD to a jack2 build directory with dbus/jackdbus}
[ -d /sys/devices/system/cpu/cpu0/cpuidle ] || { echo "SKIP: no cpuidle states on this machine"; exit 0; }
W=$HOME/.cache/patchbox-work
bin=$W/cpuwake-bin
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
rm -f "$W"/cpuwake-*.png

script=$W/cpuwake-script.txt
cat > "$script" <<'STEPS'
wait 3000
shot HOME/.cache/patchbox-work/cpuwake-1-off.png
click bar:CpuWake
wait 800
shot HOME/.cache/patchbox-work/cpuwake-2-menu.png
click menu:lowest+latency
wait 2500
shot HOME/.cache/patchbox-work/cpuwake-3-set.png
move bar:Buffer
wait 300
move bar:CpuWake
wait 1500
shot HOME/.cache/patchbox-work/cpuwake-4-tip.png
click bar:CpuWake
wait 800
click menu:off+·
wait 2500
shot HOME/.cache/patchbox-work/cpuwake-5-off-again.png
quit
STEPS
sed -i "s|HOME/|$HOME/|g" "$script"

timeout 120 "$HERE/run.sh" "$W/cpuwake-final.png" PB_BIN="$bin" PB_JACKDBUS=1 PB_JACKDBUS_BIN="$bin/jackdbus-nspa" \
  "PATCHBOX_TEST=$(grep -v '^#' "$script" | tr '\n' ';')" PATCHBOX_SNAPSHOT_DELAY_MS=60000 > "$W/cpuwake-run.log" 2>&1
fail=0
check() { if eval "$2"; then echo "ok   $1"; else echo "FAIL $1"; fail=1; fi; }
log=$W/nested/jackdbus-home/.log/jack/jackdbus.log
conf=$W/nested/jackdbus-home/.config/jack/conf.xml
check "five screenshots" 'for i in 1-off 2-menu 3-set 4-tip 5-off-again; do [ -s "$W/cpuwake-$i.png" ] || exit 1; done'
check "a level was asked for: jackdbus took it at once (held, or said why not)" 'grep -qE "Holding a CPU DMA latency request of [0-9]+ us|cpu-dma-latency: cannot" "$log"'
check "off again: reset, nothing kept in jackdbus's settings" '! grep -q "name=\"cpu-dma-latency\"" "$conf"'
grep -iE "cpu.dma" "$log" | cut -c26- | sed 's/^/    /'
echo "screenshots: $W/cpuwake-*.png"
exit $fail
