#!/bin/bash
# patchboxd end to end, against a PRIVATE jackd: unshare'd user+IPC+mount+PID namespace with its
# own tmpfs /dev/shm (the live JACK server is never visible), scratch sessions root and socket.
#   tests/session/run.sh
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
if [ -z "${PB_INNER:-}" ]; then
  exec unshare --user --map-user="$(id -u)" --map-group="$(id -g)" --keep-caps --mount --ipc --pid --fork --mount-proc \
    sh -c 'mount -t tmpfs -o size=256m,mode=1777 pb-test-shm /dev/shm && exec setpriv --inh-caps=-all --ambient-caps=-all --bounding-set=-all env PB_INNER=1 "$@"' \
    sh "$0"
fi
[ "$(findmnt -no SOURCE /dev/shm | tail -1)" = "pb-test-shm" ] && [ -z "$(ls -A /dev/shm)" ] || { echo "not isolated; refusing"; exit 1; }
W=$HOME/.cache/patchbox-work/session-test; rm -rf "$W"; mkdir -p "$W"
export PATH="$ROOT/target/debug:$PATH" PATCHBOX_SOCKET="$W/daemon.sock" PATCHBOX_SESSIONS="$W/sessions" PATCHBOXD_STAY=1
jackd --no-realtime -d dummy --rate 48000 --period 256 --capture 2 --playback 2 > "$W/jackd.log" 2>&1 &
for i in $(seq 50); do ls /dev/shm/jack_default_* >/dev/null 2>&1 && break; sleep 0.1; done; sleep 0.3
patchboxd > "$W/patchboxd.log" 2>&1 &
for i in $(seq 50); do [ -S "$W/daemon.sock" ] && break; sleep 0.1; done
fail=0
ctl() { patchbox-ctl "$@"; }
check() { if eval "$2"; then echo "ok   $1"; else echo "FAIL $1"; fail=1; fi; }
wait_for() { for i in $(seq 60); do eval "$1" && return 0; sleep 0.1; done; return 1; }
st() { ctl state; }

ctl new t1 >/dev/null
wait_for 'st | grep -q "\"busy\": null"'
check "session t1 created" 'st | grep -q "\"session\": \"t1\""'
ctl add pbx-fake-nsm >/dev/null
wait_for 'st | grep -q "\"status\": \"ready\""'
check "fake client announced and opened" 'st | grep -q "\"status\": \"ready\""'
JN=$(st | sed -n 's/.*"jack_name": "\(.*\)".*/\1/p' | head -1)
check "client got a JACK name ($JN)" '[ -n "$JN" ]'
wait_for 'ctl graph | grep -q "$JN:out_L -> system:playback_1"'
check "app auto-connected out_L" 'ctl graph | grep -q "$JN:out_L -> system:playback_1"'
ctl disconnect "$JN:out_L" system:playback_1 >/dev/null
ctl connect "$JN:out_R" system:playback_2 >/dev/null
ctl connect system:capture_1 "$JN:in_L" >/dev/null
ctl save >/dev/null
wait_for 'st | grep -q "\"busy\": null"'
check "session file written" '[ -f "$W/sessions/t1/session.toml" ]'
check "client saved its state" 'ls "$W"/sessions/t1/*/state.txt >/dev/null 2>&1'
saved_conns() { python3 -I -c 'import sys,tomllib; d=tomllib.load(open(sys.argv[1],"rb")); print("\n".join(a+" -> "+b for a,b in d["connections"]["list"]))' "$W/sessions/t1/session.toml"; }
check "connections saved (out_R, in_L; not the removed out_L)" 'saved_conns | grep -q "$JN:out_R -> system:playback_2" && saved_conns | grep -q "system:capture_1 -> $JN:in_L" && ! saved_conns | grep -q "$JN:out_L -> system:playback_1"'
check "auto snapshot taken at save" 'ctl snapshots | grep -q "saved"'
check "snapshot commits unsigned (the user's commit.gpgsign does not apply)" '[ "$(git --git-dir="$W/sessions/t1/.patchbox-snapshots" log -1 --format=%G?)" = N ]'
check "audio and logs left out of snapshots" '! git --git-dir="$W/sessions/t1/.patchbox-snapshots" ls-files | grep -q "^.patchbox/"'
ctl notes "first notes" >/dev/null
ctl snapshot with first notes >/dev/null
wait_for 'st | grep -q "\"busy\": null"'
ctl notes "second notes" >/dev/null
ctl save >/dev/null
wait_for 'st | grep -q "\"busy\": null"'
SNAP=$(ctl snapshots | grep "with first notes" | cut -f1)
check "named snapshot listed ($SNAP)" '[ -n "$SNAP" ]'
ctl rewind "$SNAP" >/dev/null
wait_for 'st | grep -q "\"busy\": null" && st | grep -q "\"status\": \"ready\""'
check "going back restored the notes" 'st | grep -q "\"notes\": \"first notes\""'
check "going back can be undone (snapshot of the state before)" 'ctl snapshots | grep -q "before going back"'
ctl close >/dev/null
wait_for 'st | grep -q "\"session\": null"'
check "closed; client stopped" '! pgrep -f pbx-fake-nsm >/dev/null'
ctl open t1 >/dev/null
wait_for 'st | grep -q "\"status\": \"ready\""'
sleep 1
G=$(ctl graph)
check "reopened: out_R restored" 'echo "$G" | grep -q "$JN:out_R -> system:playback_2"'
check "reopened: capture_1 -> in_L restored" 'echo "$G" | grep -q "system:capture_1 -> $JN:in_L"'
check "reopened: the app's auto-connect undone" '! echo "$G" | grep -q "$JN:out_L -> system:playback_1"'
ctl duplicate t2 >/dev/null
wait_for 'st | grep -q "\"session\": \"t2\"" && st | grep -q "\"busy\": null"'
check "duplicated to t2 and opened it" 'st | grep -q "\"session\": \"t2\""'
check "t1 still on disk" '[ -f "$W/sessions/t1/session.toml" ]'
check "the copy has no snapshot history of t1" '! git --git-dir="$W/sessions/t2/.patchbox-snapshots" log --format=%s 2>/dev/null | grep -q "first notes"'
check "sessions listed" 'ctl sessions | grep -q "^t2"'
ctl abort >/dev/null
wait_for 'st | grep -q "\"session\": null"'
check "aborted" 'st | grep -q "\"session\": null"'
ctl quit >/dev/null
echo "--- patchboxd log"; tail -n 25 "$W/patchboxd.log"
exit $fail
