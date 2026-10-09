#!/bin/sh
# Constant-time evidence for this crate on the current toolchain. Prints what
# was checked; exits non-zero on any failure. Evidence, not proof.
set -eu
cd "$(dirname "$0")/.."
TARGET=$(cargo metadata --format-version=1 --no-deps | python3 -c 'import json,sys;print(json.load(sys.stdin)["target_directory"])')
cargo build --release --example taint -q
BIN="$TARGET/release/examples/taint"
fail=0
. ../rusty_ct_check/scripts/taint_lib.sh
for m in seal open control; do run_mode $m; done
n=$(count seal);    echo "taint(seal):    $n error(s), want 0";   [ "$n" -eq 0 ] || fail=1
n=$(count open);    echo "taint(open):    $n error(s), want exactly 1 (the public verdict)"; [ "$n" -eq 1 ] || fail=1
report open | grep -q 'open_in_place' || { echo "taint(open): the one report is not in open_in_place"; fail=1; }
n=$(count control); echo "taint(control): $n error(s), want >=1"; [ "$n" -ge 1 ] || fail=1
python3 -I ../rusty_ct_check/scripts/disasm_audit.py "$BIN" scripts/disasm_limits.txt || fail=1
exit $fail
