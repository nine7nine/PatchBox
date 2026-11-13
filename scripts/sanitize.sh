#!/usr/bin/env bash
# Sanitizer builds of PatchBox (nightly Rust): AddressSanitizer (with LeakSanitizer) or
# ThreadSanitizer, the standard library rebuilt with the same instrumentation (-Zbuild-std, needs
# rust-src), into target/<sanitizer>/ so the normal build is left alone. The C parts (our static
# SDL, the system's libjack) are not instrumented.
#
#   scripts/sanitize.sh address|thread           build
#   PB_BIN=$(scripts/sanitize.sh address --bin-dir) tests/session/run.sh
#                                                 the session tests against that build
#   tests/nested/run.sh out.png PB_BIN=…          the GUI harness against it
set -euo pipefail
san=${1:?address or thread}
shift
cd "$(dirname "$0")/.."
triple=x86_64-unknown-linux-gnu
dir="$PWD/target/$san/$triple/debug"
if [ "${1:-}" = --bin-dir ]; then
  echo "$dir"
  exit 0
fi
case "$san" in address | thread) ;; *) echo "address or thread" >&2; exit 2 ;; esac
export RUSTFLAGS="-Zsanitizer=$san -Cforce-frame-pointers=yes" RUSTDOCFLAGS="-Zsanitizer=$san"
cargo +nightly build -Zbuild-std --target "$triple" --target-dir "target/$san" --workspace --bins "$@"
echo "built in $dir"
