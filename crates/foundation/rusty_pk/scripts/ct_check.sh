#!/bin/sh
# Constant-time evidence for the secret-scalar code (X25519) on the current
# toolchain. Prints what was checked; exits non-zero on any failure.
# Evidence, not proof.
set -eu
cd "$(dirname "$0")/.."
TARGET=$(cargo metadata --format-version=1 --no-deps | python3 -c 'import json,sys;print(json.load(sys.stdin)["target_directory"])')
cargo build --release --example taint -q
BIN="$TARGET/release/examples/taint"
fail=0
. ../rusty_ct_check/scripts/taint_lib.sh
for m in x25519 agree control; do run_mode $m; done
n=$(count x25519);  echo "taint(x25519):  $n error(s), want 0";   [ "$n" -eq 0 ] || fail=1
n=$(count agree);   echo "taint(agree):   $n error(s), want exactly 1 (peer small-order verdict)"; [ "$n" -eq 1 ] || fail=1
report agree | grep -q 'x25519::agree' || { echo "taint(agree): the one report is not in agree"; fail=1; }
n=$(count control); echo "taint(control): $n error(s), want >=1"; [ "$n" -ge 1 ] || fail=1
python3 -I ../rusty_ct_check/scripts/disasm_audit.py "$BIN" scripts/disasm_limits.txt || fail=1
exit $fail
