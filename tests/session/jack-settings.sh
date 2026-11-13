#!/bin/bash
# Per-session JACK settings through jackdbus, end to end: a private session bus
# (tests/nested/bus.conf), a private namespace with its own /dev/shm, jackdbus with a scratch
# HOME (its conf.xml never touches the owner's), patchboxd with scratch sessions.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
if [ -z "${PB_BUS:-}" ]; then
  exec env PB_BUS=1 dbus-run-session --config-file="$ROOT/tests/nested/bus.conf" -- "$0"
fi
if [ -z "${PB_INNER:-}" ]; then
  exec unshare --user --map-user="$(id -u)" --map-group="$(id -g)" --keep-caps --mount --ipc --pid --fork --mount-proc \
    sh -c 'mount -t tmpfs -o size=256m,mode=1777 pb-test-shm /dev/shm && exec setpriv --inh-caps=-all --ambient-caps=-all --bounding-set=-all env PB_INNER=1 "$@"' \
    sh "$0"
fi
[ "$(findmnt -no SOURCE /dev/shm | tail -1)" = "pb-test-shm" ] && [ -z "$(ls -A /dev/shm)" ] || { echo "not isolated; refusing"; exit 1; }
W=$HOME/.cache/patchbox-work/jack-settings-test; rm -rf "$W"; mkdir -p "$W/home/.config"
export HOME="$W/home" XDG_CONFIG_HOME="$W/home/.config"
export PATH="$ROOT/target/debug:$PATH" PATCHBOX_SOCKET="$W/daemon.sock" PATCHBOX_SESSIONS="$W/sessions" PATCHBOX_TEMPLATES="$W/templates" PATCHBOXD_STAY=1 PATCHBOX_CONFIG_DIR="$W/config"
fail=0
check() { if eval "$2"; then echo "ok   $1"; else echo "FAIL $1"; fail=1; fi; }
wait_for() { for i in $(seq 100); do eval "$1" && return 0; sleep 0.1; done; return 1; }
jackdbus auto > "$W/jackdbus.log" 2>&1 &
wait_for 'jack_control status >/dev/null 2>&1'
{ jack_control ds dummy && jack_control dps rate 48000 && jack_control dps period 256 && jack_control eps realtime false && jack_control start; } > "$W/jack_control.log" 2>&1
check "jackdbus started the server (dummy, 48 kHz)" 'jack_control status 2>/dev/null | grep -q started'
patchboxd > "$W/patchboxd.log" 2>&1 &
wait_for '[ -S "$W/daemon.sock" ]'
st() { patchbox-ctl state; }
patchbox-ctl new j1 >/dev/null; wait_for 'st | grep -q "\"busy\": null"'
patchbox-ctl jack-memory on >/dev/null
patchbox-ctl save >/dev/null; wait_for 'st | grep -q "\"busy\": null"'
check "the session keeps JACK's settings" 'grep -A8 "^\[jack\]" "$W/sessions/j1/session.toml" | grep -q "driver = \"dummy\"" && grep -q "u:48000" "$W/sessions/j1/session.toml"'
# The session closes (keeping 48 kHz); then JACK is changed outside it.
patchbox-ctl close >/dev/null; wait_for 'st | grep -q "\"session\": null"'
{ jack_control dps rate 44100 && jack_control stop && jack_control start; } >> "$W/jack_control.log" 2>&1
rate() { jack_control dp 2>/dev/null | grep -E '^ *rate:' ; }
rate > "$W/rate-before.txt"
check "JACK now runs at 44.1 kHz" 'rate | grep -q "44100)"'
patchbox-ctl open j1 >/dev/null; wait_for 'st | grep -q "\"session\": \"j1\"" && st | grep -q "\"busy\": null"'
check "opening the session put 48 kHz back" 'rate | grep -q "48000)"'
check "and the server runs" 'jack_control status 2>/dev/null | grep -q started'
check "patchboxd says so" 'grep -q "JACK restarted with the session" "$W/patchboxd.log"'
patchbox-ctl quit >/dev/null
exit $fail
