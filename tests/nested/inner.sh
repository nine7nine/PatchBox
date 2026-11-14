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
# XDG_DATA_HOME: deleting moves things to a scratch Trash, never the user's.
export PATH="$BIN:$PATH" PATCHBOX_SOCKET="$L/daemon.sock" PATCHBOX_SESSIONS="$L/sessions" XDG_DATA_HOME="$L/xdg-data" PATCHBOX_TEMPLATES="$L/templates"
rm -rf "$L/sessions" "$L/xdg-data" "$L/templates"
if [ -n "${PB_JACKDBUS:-}" ]; then
  # jackdbus on run.sh's private session bus, with a scratch HOME: its conf.xml and logs never
  # touch the user's ~/.config/jack.
  JH=$L/jackdbus-home; rm -rf "$JH"; mkdir -p "$JH/.config"
  # PB_JACKDBUS_BIN: another jackdbus (e.g. a wrapper running a jack2 build), default the PATH's.
  env HOME="$JH" XDG_CONFIG_HOME="$JH/.config" "${PB_JACKDBUS_BIN:-jackdbus}" auto > "$L/jackdbus.log" 2>&1 &
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
if [ -n "${PB_PIPEWIRE:-}" ]; then
  # A private PipeWire: its socket in a scratch runtime dir that pw-dump/pw-link use too.
  # (-c with an absolute path: none of the user's PipeWire configuration applies to the daemon.)
  export PIPEWIRE_RUNTIME_DIR="$L/pw-run"
  rm -rf "$PIPEWIRE_RUNTIME_DIR"; mkdir -p "$PIPEWIRE_RUNTIME_DIR"
  # PB_PIPEWIRE=bridge: PipeWire on top of the JACK server above (module-jack-tunnel), as with
  # jack2dbus-nspa + module-jackdbus-detect; anything else: PipeWire alone.
  conf=pw-test.conf; [ "$PB_PIPEWIRE" = bridge ] && conf=pw-bridge.conf
  env HOME="$L/pw-home" pipewire -c "$(dirname "$0")/$conf" > "$L/pipewire.log" 2>&1 &
  for i in $(seq 50); do [ -S "$PIPEWIRE_RUNTIME_DIR/pipewire-0" ] && break; sleep 0.1; done
  sleep 0.3
  timeout 5 pw-link firefox:capture_FL skype_in:playback_FL >/dev/null 2>&1
  timeout 5 pw-link firefox:capture_FR jack_sink:playback_FR >/dev/null 2>&1
fi
if [ -z "${PB_NO_FAKES:-}" ]; then
  "$BIN/pbx-fake-clients" > "$L/fakes.log" 2>&1 &
  for i in $(seq 50); do grep -q ready "$L/fakes.log" 2>/dev/null && break; sleep 0.1; done
fi
# PB_OLD_DAEMON=<patchboxd>: an older daemon already running when the window starts (as after an
# update with a session open); PB_OLD_SETUP: shell commands run against it first (patchbox-ctl …).
if [ -n "${PB_OLD_DAEMON:-}" ]; then
  "$PB_OLD_DAEMON" > "$L/old-daemon.log" 2>&1 &
  for i in $(seq 50); do [ -S "$PATCHBOX_SOCKET" ] && break; sleep 0.1; done
  [ -n "${PB_OLD_SETUP:-}" ] && sh -c "$PB_OLD_SETUP" > "$L/old-setup.log" 2>&1
fi
# PB_SND_UNPLUG=<card>:<out s>:<back s> (with PB_SND_FAKE): the card's /dev/snd links are removed
# <out s> after the window starts and restored <back s> later (its control node last, as the
# kernel registers it).
if [ -n "${PB_SND_UNPLUG:-}" ] && [ -n "${PB_SND_FAKE:-}" ]; then
  ucard=${PB_SND_UNPLUG%%:*}; rest=${PB_SND_UNPLUG#*:}; uout=${rest%%:*}; uback=${rest#*:}
  uidx=$(readlink "/proc/asound/$ucard" | sed 's/^card//')
  (
    sleep "$uout"
    for n in "$PB_REALSND"/*C"$uidx"D* "$PB_REALSND/controlC$uidx"; do rm -f "/dev/snd/$(basename "$n")"; done
    echo "unplugged $ucard"
    sleep "$uback"
    for n in "$PB_REALSND"/*C"$uidx"D*; do ln -s "$n" /dev/snd/; done
    ln -s "$PB_REALSND/controlC$uidx" /dev/snd/
    echo "plugged $ucard back"
  ) > "$L/unplug.log" 2>&1 &
fi
# Its own XDG config home: the PipeWire tab's drop-in and config lookups stay in the scratch dir.
mkdir -p "$L/xdg-config"
env PATCHBOX_CONFIG_DIR="$CFG" XDG_CONFIG_HOME="$L/xdg-config" PATCHBOX_SNAPSHOT="$OUT" PATCHBOX_SNAPSHOT_EXIT=1 PATCHBOX_SHADOW=0 "$@" "$BIN/patchbox" 2>&1 | tee "$L/patchbox.log"
