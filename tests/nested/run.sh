#!/bin/bash
# PatchBox against a PRIVATE JACK server, shown in a nested KWin, never the desktop's.
#
#   tests/nested/run.sh [out.png] [extra env assignments...]
#
# - A nested `kwin_wayland --virtual` on its own socket, under a private session bus
#   (dbus-run-session with bus.conf: no activatable services).
# - Inside an unshare'd user+IPC+mount+PID namespace with its own tmpfs on /dev/shm: jackd with
#   the dummy driver, pbx-fake-clients, then PatchBox with PATCHBOX_SNAPSHOT. JACK's shm
#   registry, SysV lock and sockets all live in that namespace: the live jackdbus server and its
#   /dev/shm segments are never visible, let alone touched.
# - PatchBox's config goes to a scratch dir (PATCHBOX_CONFIG_DIR), not ~/.config/patchbox.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
OUT=${1:-$HOME/.cache/patchbox-work/shot.png}
shift || true
if [ -z "${PB_IN_BUS:-}" ]; then
  exec env PB_IN_BUS=1 dbus-run-session --config-file="$HERE/bus.conf" -- "$0" "$OUT" "$@"
fi
# PB_* assignments configure the harness itself (PB_JACKDBUS=1, PB_NO_FAKES=1, PB_BIN=...).
for a in "$@"; do case "$a" in PB_*=*) export "$a" ;; esac; done
W=$HOME/.cache/patchbox-work/nested
mkdir -p "$W"
export WAYLAND_DISPLAY=
XDG_CONFIG_HOME=$W/kwin-config KWIN_WAYLAND_NO_PERMISSION_CHECKS=1 \
  kwin_wayland --virtual --socket pb-nested --width ${PB_W:-1440} --height ${PB_H:-900} \
  --no-lockscreen --no-global-shortcuts > "$W/kwin.log" 2>&1 &
k=$!
for i in $(seq 100); do [ -S "$XDG_RUNTIME_DIR/pb-nested" ] && break; sleep 0.1; done
[ -S "$XDG_RUNTIME_DIR/pb-nested" ] || { echo "nested kwin did not start"; cat "$W/kwin.log"; kill $k; exit 1; }
export WAYLAND_DISPLAY=pb-nested
CFG=$W/config; rm -rf "$CFG"; mkdir -p "$CFG"
[ -n "${PB_LAYOUT:-}" ] && cp "$PB_LAYOUT" "$CFG/layout.json"
unshare --user --map-user="$(id -u)" --map-group="$(id -g)" --keep-caps --mount --ipc --pid --fork --mount-proc \
  sh -c 'mount -t tmpfs -o size=256m,mode=1777 pb-test-shm /dev/shm && exec setpriv --inh-caps=-all --ambient-caps=-all --bounding-set=-all "$@"' \
  sh "$HERE/inner.sh" "$ROOT" "$OUT" "$CFG" "$@"
rc=$?
kill $k 2>/dev/null; wait $k 2>/dev/null
exit $rc
