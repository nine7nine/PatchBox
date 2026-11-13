#!/bin/bash
# Runs inside run.sh's private namespace (own /dev/shm, IPC, PIDs). Never run it directly.
set -u
ROOT=$1 OUT=$2 CFG=$3; shift 3
# The topmost mount on /dev/shm must be ours, and empty (no jack-shm-registry from the live server).
[ "$(findmnt -no SOURCE /dev/shm | tail -1)" = "pb-test-shm" ] && [ -z "$(ls -A /dev/shm)" ] \
  || { echo "inner.sh: /dev/shm is not the private, empty tmpfs; refusing"; exit 1; }
BIN=${PB_BIN:-$ROOT/target/debug}
L=$HOME/.cache/patchbox-work/nested
# The session daemon the GUI starts: scratch socket and sessions; test clients on PATH.
export PATH="$BIN:$PATH" PATCHBOX_SOCKET="$L/daemon.sock" PATCHBOX_SESSIONS="$L/sessions"
rm -rf "$L/sessions"
if [ -n "${PB_JACKDBUS:-}" ]; then
  # jackdbus on run.sh's private session bus, with a scratch HOME: its conf.xml and logs never
  # touch the owner's ~/.config/jack.
  JH=$L/jackdbus-home; rm -rf "$JH"; mkdir -p "$JH/.config"
  env HOME="$JH" XDG_CONFIG_HOME="$JH/.config" jackdbus auto > "$L/jackdbus.log" 2>&1 &
  for i in $(seq 50); do HOME="$JH" jack_control status >/dev/null 2>&1 && break; sleep 0.1; done
  ( export HOME="$JH" XDG_CONFIG_HOME="$JH/.config"
    jack_control ds dummy && jack_control dps rate 48000 && jack_control dps period 256 \
      && jack_control dps capture 4 && jack_control dps playback 4 && jack_control eps realtime false \
      && jack_control start ) > "$L/jack_control.log" 2>&1
else
  jackd --no-realtime -d dummy --rate 48000 --period 256 --capture 4 --playback 4 > "$L/jackd.log" 2>&1 &
fi
for i in $(seq 50); do ls /dev/shm/jack_default_* >/dev/null 2>&1 && break; sleep 0.1; done
sleep 0.3
if [ -z "${PB_NO_FAKES:-}" ]; then
  "$BIN/pbx-fake-clients" > "$L/fakes.log" 2>&1 &
  for i in $(seq 50); do grep -q ready "$L/fakes.log" 2>/dev/null && break; sleep 0.1; done
fi
env PATCHBOX_CONFIG_DIR="$CFG" PATCHBOX_SNAPSHOT="$OUT" PATCHBOX_SNAPSHOT_EXIT=1 PATCHBOX_SHADOW=0 "$@" "$BIN/patchbox" 2>&1 | tee "$L/patchbox.log"
