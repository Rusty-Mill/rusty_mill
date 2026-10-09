#!/bin/sh
# Constant-time evidence for the secret-scalar code (X25519, ECDH P-256/P-384) on the current
# toolchain. Prints what was checked; exits non-zero on any failure.
# Evidence, not proof.
set -eu
cd "$(dirname "$0")/.."
TARGET=$(cargo metadata --format-version=1 --no-deps | python3 -c 'import json,sys;print(json.load(sys.stdin)["target_directory"])')
cargo build --release --example taint -q
BIN="$TARGET/release/examples/taint"
fail=0
. ../rusty_ct_check/scripts/taint_lib.sh
for m in x25519 agree control ecdh256 ecdh384 ecdh-control; do run_mode $m; done
n=$(count x25519);  echo "taint(x25519):  $n error(s), want 0";   [ "$n" -eq 0 ] || fail=1
n=$(count agree);   echo "taint(agree):   $n error(s), want exactly 1 (peer small-order verdict)"; [ "$n" -eq 1 ] || fail=1
report agree | grep -q 'x25519::agree' || { echo "taint(agree): the one report is not in agree"; fail=1; }
n=$(count control); echo "taint(control): $n error(s), want >=1"; [ "$n" -ge 1 ] || fail=1
# ECDH: exactly two reports per curve, both the 'result is the point at infinity?' test in public_key
# and agree, which branches on a value derived from the secret but is false for every valid key.
for m in ecdh256 ecdh384; do
  n=$(count $m); echo "taint($m): $n error(s), want exactly 2 (infinity test in public_key and agree)"; [ "$n" -eq 2 ] || fail=1
  [ "$(report $m | grep -c 'ecdh::PrivateKey>::public_key')" -eq 1 ] || { echo "taint($m): no single report in public_key"; fail=1; }
  [ "$(report $m | grep -c 'ecdh::PrivateKey>::agree')" -eq 1 ] || { echo "taint($m): no single report in agree"; fail=1; }
done
n=$(count ecdh-control); echo "taint(ecdh-control): $n error(s), want >=2 (agree's plus the planted branch)"; [ "$n" -ge 2 ] || fail=1
report ecdh-control | grep -q 'at 0x[0-9A-F]*: taint::main' || { echo "taint(ecdh-control): the planted branch was not reported in main"; fail=1; }
python3 -I ../rusty_ct_check/scripts/disasm_audit.py "$BIN" scripts/disasm_limits.txt || fail=1
exit $fail
