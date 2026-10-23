#!/bin/sh
# Constant-time evidence for this crate on the current toolchain. Prints what
# was checked; exits non-zero on any failure. Evidence, not proof.
set -eu
cd "$(dirname "$0")/.."
TARGET=$(cargo metadata --format-version=1 --no-deps | python3 -c 'import json,sys;print(json.load(sys.stdin)["target_directory"])')
cargo build --release --example rusty_aead_taint -q
BIN="$TARGET/release/examples/rusty_aead_taint"
fail=0
. ../rusty_ct_check/scripts/taint_lib.sh
for m in seal open control; do run_mode $m; done
n=$(count seal);    echo "taint(seal):    $n error(s), want 0";   [ "$n" -eq 0 ] || fail=1
n=$(count open);    echo "taint(open):    $n error(s), want exactly 1 (the public verdict)"; [ "$n" -eq 1 ] || fail=1
report open | grep -q 'open_in_place' || { echo "taint(open): the one report is not in open_in_place"; fail=1; }
n=$(count control); echo "taint(control): $n error(s), want >=1"; [ "$n" -ge 1 ] || fail=1

# AES-GCM (key and plaintext secret; the key schedule runs under the secret key too). `open` has exactly one report:
# the public tag verdict in Gcm::open. `control-table` is a secret-indexed table lookup, the leak table-based AES
# has, and must be reported or a clean result above would mean nothing.
cargo build --release --example taint_aes -q
BIN_AES="$TARGET/release/examples/taint_aes"
BIN_CHACHA="$BIN"
BIN="$BIN_AES"
for m in gcm128-seal gcm256-seal gcm128-open control-table; do run_mode $m; done
for m in gcm128-seal gcm256-seal; do
  n=$(count $m); echo "taint($m): $n error(s), want 0"; [ "$n" -eq 0 ] || fail=1
done
n=$(count gcm128-open); echo "taint(gcm128-open): $n error(s), want exactly 1 (the public verdict)"; [ "$n" -eq 1 ] || fail=1
report gcm128-open | grep -q 'Gcm>::open' || { echo "taint(gcm128-open): the one report is not in Gcm::open"; fail=1; }
n=$(count control-table); echo "taint(control-table): $n error(s), want >=1"; [ "$n" -ge 1 ] || fail=1

python3 -I ../rusty_ct_check/scripts/disasm_audit.py "$BIN_CHACHA" scripts/disasm_limits.txt || fail=1
python3 -I ../rusty_ct_check/scripts/disasm_audit.py "$BIN_AES" scripts/disasm_limits_aes.txt || fail=1
exit $fail
