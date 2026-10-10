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
for m in x25519 agree control ecdh256 ecdh384 ecdh-import256 ecdh-import384 ecdh-generate256 ecdh-generate384 ecdh-control; do run_mode $m; done
n=$(count x25519);  echo "taint(x25519):  $n error(s), want 0";   [ "$n" -eq 0 ] || fail=1
n=$(count agree);   echo "taint(agree):   $n error(s), want exactly 1 (peer small-order verdict)"; [ "$n" -eq 1 ] || fail=1
report agree | grep -q 'x25519::agree' || { echo "taint(agree): the one report is not in agree"; fail=1; }
n=$(count control); echo "taint(control): $n error(s), want >=1"; [ "$n" -ge 1 ] || fail=1
# ECDH: the scalar is marked secret BEFORE import (so range validation, public_key, agree and generate are
# all inside the taint). The only allowed reports are the branch on a final verdict (scalar valid; result
# not the point at infinity): a conditional jump right after a call to ecdh::ensure, one per call site
# (from_bytes, public_key, agree). verdict_sites.py rejects a report anywhere else, e.g. in a limb loop.
for m in ecdh256 ecdh384 ecdh-import256 ecdh-import384 ecdh-generate256 ecdh-generate384; do
  case $m in ecdh256|ecdh384) want=3 ;; *) want=1 ;; esac
  n=$(count $m); echo "taint($m): $n error(s), want exactly $want (verdict branches only)"; [ "$n" -eq $want ] || fail=1
  python3 -I scripts/verdict_sites.py "$BIN" "$OUT/$m" $want || fail=1
done
n=$(count ecdh-control); echo "taint(ecdh-control): $n error(s), want >=3 (verdicts plus the planted branch)"; [ "$n" -ge 3 ] || fail=1
report ecdh-control | grep -q 'at 0x[0-9A-F]*: taint::main' || { echo "taint(ecdh-control): the planted branch was not reported in main"; fail=1; }
if python3 -I scripts/verdict_sites.py "$BIN" "$OUT/ecdh-control" >/dev/null; then echo "taint(ecdh-control): verdict_sites accepted the planted branch"; fail=1; fi
python3 -I ../rusty_ct_check/scripts/disasm_audit.py "$BIN" scripts/disasm_limits.txt || fail=1
exit $fail
