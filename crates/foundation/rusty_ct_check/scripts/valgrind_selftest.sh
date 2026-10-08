#!/bin/sh
# Shows the taint tool still catches planted leaks: `clean` must report 0
# errors, the two leaky modes at least 1. Exits non-zero otherwise.
set -eu
cd "$(dirname "$0")/.."
cargo build --release --example probe -q
BIN=$(cargo metadata --format-version=1 --no-deps | python3 -c 'import json,sys;print(json.load(sys.stdin)["target_directory"])')/release/examples/probe
errors() { valgrind -q --error-exitcode=0 "$BIN" "$1" 2>&1 | grep -cE '^==[0-9]+== (Conditional|Use of)' || true; }
fail=0
c=$(errors clean);        [ "$c" -eq 0 ] || { echo "FAIL clean reported $c"; fail=1; }
for m in leaky-branch leaky-index; do
  c=$(errors $m);         [ "$c" -ge 1 ] || { echo "FAIL $m reported 0 (leak not caught)"; fail=1; }
done
[ "$fail" -eq 0 ] && echo "ok: taint tool catches planted leaks and passes the clean probe"
exit $fail
