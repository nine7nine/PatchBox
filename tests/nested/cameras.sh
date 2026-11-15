#!/bin/bash
# Views: each graph (JACK, PipeWire) keeps its own view style (Ports, vertical Ports, Matrix), and
# each graph and session canvas keeps, per style, its own camera and block/matrix positions,
# across style and graph switches, Fit, sessions and restarts. Runs the scripts in cameras/ in the
# nested harness (private JACK + PipeWire bridge) and checks every "cam =X" line equals "cam X"
# (graph, style, camera, position fingerprints).
#   tests/nested/cameras.sh            (PB_BIN defaults to target/release)
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
BIN=${PB_BIN:-$ROOT/target/release}
fail=0
run() { # script [extra harness args…] -> its "cam" lines
  local s=$1; shift
  local steps; steps=$(grep -v '^#' "$HERE/cameras/$s" | paste -sd';')
  timeout 180 "$HERE/run.sh" "$HOME/.cache/patchbox-work/cameras.png" PB_BIN="$BIN" PB_PIPEWIRE=bridge "$@" \
    PATCHBOX_SNAPSHOT_DELAY_MS=120000 "PATCHBOX_TEST=$steps" 2>&1 | grep -E '^\[test\] (cam|cannot resolve)' | sed 's/^\[test\] //'
}
check() { # name, cam lines: every =X matches X (graph, style, position, zoom)
  local name=$1 lines=$2
  if echo "$lines" | grep -q "cannot resolve"; then echo "FAIL $name: $(echo "$lines" | grep "cannot resolve" | head -1)"; fail=1; return; fi
  local bad
  bad=$(echo "$lines" | python3 -I -c '
import sys
seen = {}
bad = []
for line in sys.stdin:
    w = line.split()
    if len(w) < 6 or w[0] != "cam":
        continue
    label, rest = w[1], " ".join(w[2:])
    if label.startswith("="):
        want = seen.get(label[1:])
        if want != rest:
            bad.append(f"{label}: {rest} (want {want})")
    else:
        seen[label] = rest
print("\n".join(bad))')
  local n; n=$(echo "$lines" | grep -c '^cam =')
  if [ -n "$bad" ]; then echo "FAIL $name:"; echo "$bad" | sed 's/^/     /'; fail=1; else echo "ok   $name ($n returns checked)"; fi
}
check "switching graphs and styles (Ports, vertical, Matrix; Fit; moved blocks)" "$(run switch.txt)"
check "a style a graph has never shown is fitted, not borrowed" "$(run unseen.txt)"
P1=$(run persist1.txt)
P2=$(run persist2.txt PB_KEEP_CFG=1)
check "everything back after a restart from PipeWire" "$(printf '%s\n%s\n' "$P1" "$P2")"
check "session canvases, and sessions changed while on PipeWire" "$(run sessions.txt)"
check "styles inside sessions" "$(run session-styles.txt)"
exit $fail
