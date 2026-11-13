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
export PATH="$ROOT/target/debug:$PATH" PATCHBOX_SOCKET="$W/daemon.sock" PATCHBOX_SESSIONS="$W/sessions" PATCHBOXD_STAY=1 PATCHBOX_TEMPLATES="$W/templates" PATCHBOX_CONFIG_DIR="$W/config"
jackd --no-realtime -d dummy --rate 48000 --period 256 --capture 2 --playback 2 > "$W/jackd.log" 2>&1 &
for i in $(seq 50); do ls /dev/shm/jack_default_* >/dev/null 2>&1 && break; sleep 0.1; done; sleep 0.3
# PB_VALGRIND=1: the daemon under memcheck (invalid reads/writes, uninitialised values in the FFI
# and libc code); errors fail the run.
if [ -n "${PB_VALGRIND:-}" ]; then
  # Needs a dynamic linker valgrind can read (glibc debug info); say so instead of failing later.
  valgrind -q --error-exitcode=99 true 2>"$W/valgrind-check.log" || { echo "valgrind cannot run here:"; grep -m3 -E 'Fatal|install' "$W/valgrind-check.log"; exit 2; }
  valgrind --error-exitcode=99 --track-origins=yes --leak-check=no --log-file="$W/valgrind.log" "$ROOT/target/debug/patchboxd" > "$W/patchboxd.log" 2>&1 &
else
  patchboxd > "$W/patchboxd.log" 2>&1 &
fi
VPID=$!
for i in $(seq 200); do [ -S "$W/daemon.sock" ] && break; sleep 0.1; done
fail=0
ctl() { patchbox-ctl "$@"; }
check() { if eval "$2"; then echo "ok   $1"; else echo "FAIL $1"; fail=1; fi; }
wait_for() { for i in $(seq ${PB_WAIT:-60}); do eval "$1" && return 0; sleep 0.1; done; return 1; }
st() { ctl state; }

# A load hook for every session (sessions root): it records that it ran, and with what.
mkdir -p "$W/sessions/patchbox-scripts"
cat > "$W/sessions/patchbox-scripts/load.sh" <<'SH'
echo "$PATCHBOX_SCRIPT $PATCHBOX_SESSION_NAME" >> "$PATCHBOX_SESSION_PATH/script-ran"
SH
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
check "the load script ran for t2" 'grep -q "load t2" "$W/sessions/t2/script-ran"'
PID2=$(st | sed -n 's/.*"pid": \([0-9]*\).*/\1/p' | head -1)
ctl open t1 >/dev/null
wait_for 'st | grep -q "\"session\": \"t1\"" && st | grep -q "\"busy\": null"'
PID1=$(st | sed -n 's/.*"pid": \([0-9]*\).*/\1/p' | head -1)
check "switch t2 -> t1 kept the :switch: client running (pid $PID2 -> $PID1)" '[ -n "$PID2" ] && [ "$PID1" = "$PID2" ] && st | grep -q "\"status\": \"ready\""'
check "the switched client was told its new place" 'ls -d "$W"/sessions/t1/FakeApp.* >/dev/null 2>&1'
ctl open t2 >/dev/null
wait_for 'st | grep -q "\"session\": \"t2\"" && st | grep -q "\"busy\": null"'
check "the copy has no snapshot history of t1" '! git --git-dir="$W/sessions/t2/.patchbox-snapshots" log --format=%s 2>/dev/null | grep -q "first notes"'
check "sessions listed" 'ctl sessions | grep -q "^t2"'
export PATCHBOX_TEMPLATES="$W/templates"
ctl save-template band >/dev/null
wait_for 'st | grep -q "\"busy\": null"'
check "saved as template, without snapshots" '[ -f "$W/templates/band/session.toml" ] && [ ! -d "$W/templates/band/.patchbox-snapshots" ]'
ctl new from-band --template band >/dev/null
wait_for 'st | grep -q "\"session\": \"from-band\"" && st | grep -q "\"status\": \"ready\""'
check "new session from the template runs its client" 'st | grep -q "\"session\": \"from-band\"" && st | grep -q "\"status\": \"ready\""'
check "the template's connections came along" 'grep -q "out_R" "$W/sessions/from-band/session.toml"'
# A plain program: $CLIENT_ID in its arguments, an environment variable, a save signal.
cat > "$W/plain.sh" <<'SH'
#!/bin/sh
echo "$1 $FOO" > "$PATCHBOX_SESSION_PATH/plain-args"
trap 'echo saved > "$PATCHBOX_SESSION_PATH/plain-saved"' USR1
while :; do sleep 0.1; done
SH
chmod +x "$W/plain.sh"
ctl add --plain "$W/plain.sh" '$CLIENT_ID' >/dev/null
PL=$(st | python3 -I -c 'import sys,json; print(next(c["id"] for c in json.load(sys.stdin)["clients"] if c["exe"].endswith("plain.sh")))')
ctl props "$PL" save=USR1 env=FOO=bar >/dev/null
ctl stop "$PL" >/dev/null
wait_for 'st | python3 -I -c "import sys,json; sys.exit(0 if any(c[\"id\"]==\"$PL\" and c[\"status\"]==\"stopped\" for c in json.load(sys.stdin)[\"clients\"]) else 1)"'
rm -f "$W/sessions/from-band/plain-args"
ctl start "$PL" >/dev/null
wait_for '[ -s "$W/sessions/from-band/plain-args" ]'
check "a plain program gets \$CLIENT_ID in its arguments and its environment" 'grep -qx "$PL bar" "$W/sessions/from-band/plain-args"'
ctl save >/dev/null
wait_for '[ -f "$W/sessions/from-band/plain-saved" ]'
check "a plain program saves on its signal" '[ -f "$W/sessions/from-band/plain-saved" ]'
# A client template: the fake NSM client with its files, added back as a new client; a per-client
# start hook (sessions root) runs for it.
mkdir -p "$W/sessions/patchbox-scripts/clients"
echo 'echo "$PATCHBOX_CLIENT_PID" > "$PATCHBOX_SESSION_PATH/hook-start-$PATCHBOX_CLIENT_ID"' > "$W/sessions/patchbox-scripts/clients/start.sh"
ids() { st | python3 -I -c 'import sys,json; print(" ".join(c["id"] for c in json.load(sys.stdin)["clients"] if c["nsm"]))'; }
FK=$(ids | cut -d' ' -f1)
ctl save-client-template "$FK" fake-tpl >/dev/null
check "client template kept with its files" '[ -f "$W/config/client-templates/fake-tpl/client.toml" ] && ls "$W"/config/client-templates/fake-tpl/files/__CLIENT__*/state.txt >/dev/null 2>&1'
check "client templates listed" 'ctl client-templates | grep -qx fake-tpl'
ctl add-template fake-tpl >/dev/null
wait_for '[ "$(ids | wc -w)" -ge 2 ] && [ "$(st | grep -c "\"status\": \"ready\"")" -ge 2 ]'
NEW=$(ids | tr ' ' '\n' | grep -vx "$FK" | head -1)
check "a client added from the template gets its own copy of the files ($NEW)" 'ls "$W"/sessions/from-band/*."$NEW"/state.txt >/dev/null 2>&1'
wait_for '[ -s "$W/sessions/from-band/hook-start-$NEW" ]'
check "the per-client start hook ran for it" '[ -s "$W/sessions/from-band/hook-start-$NEW" ]'
ctl abort >/dev/null
wait_for 'st | grep -q "\"session\": null"'
check "aborted" 'st | grep -q "\"session\": null"'
ctl quit >/dev/null
wait $VPID; DRC=$?
if [ -n "${PB_VALGRIND:-}" ]; then
  check "valgrind: no memory errors in patchboxd (exit $DRC)" '[ $DRC -ne 99 ] && grep -q "ERROR SUMMARY: 0 errors" "$W/valgrind.log"'
  grep -E "ERROR SUMMARY" "$W/valgrind.log"
fi
echo "--- patchboxd log"; tail -n 25 "$W/patchboxd.log"
exit $fail
