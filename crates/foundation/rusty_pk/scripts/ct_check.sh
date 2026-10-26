#!/bin/sh
# Constant-time evidence for the secret-scalar code (X25519, ECDH P-256/P-384, Ed25519 and ECDSA signing) on the current
# toolchain. Prints what was checked; exits non-zero on any failure.
# Evidence, not proof.
set -eu
cd "$(dirname "$0")/.."
TARGET=$(cargo metadata --format-version=1 --no-deps | python3 -c 'import json,sys;print(json.load(sys.stdin)["target_directory"])')
cargo build --release --example rusty_pk_taint -q
BIN="$TARGET/release/examples/rusty_pk_taint"
fail=0
. ../rusty_ct_check/scripts/taint_lib.sh
for m in x25519 agree control ecdh256 ecdh384 ecdh-import256 ecdh-import384 ecdh-generate256 ecdh-generate384 ecdh-control ed25519-sign ed25519-control ecdsa-sign256 ecdsa-sign384 ecdsa-control pkcs8-ecdsa256 pkcs8-ed25519; do run_mode $m; done
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
report ecdh-control | grep -q 'at 0x[0-9A-F]*: rusty_pk_taint::main' || { echo "taint(ecdh-control): the planted branch was not reported in main"; fail=1; }
if python3 -I scripts/verdict_sites.py "$BIN" "$OUT/ecdh-control" >/dev/null; then echo "taint(ecdh-control): verdict_sites accepted the planted branch"; fail=1; fi
# Ed25519 signing: the seed is secret before key expansion; expansion, both fixed-base multiplications,
# the reductions and S = r + k*s must not branch on it at all.
n=$(count ed25519-sign); echo "taint(ed25519-sign): $n error(s), want 0"; [ "$n" -eq 0 ] || fail=1
n=$(count ed25519-control); echo "taint(ed25519-control): $n error(s), want >=1 (the planted branch)"; [ "$n" -ge 1 ] || fail=1
report ed25519-control | grep -q 'rusty_pk_taint::main' || { echo "taint(ed25519-control): the planted branch was not reported in main"; fail=1; }
# ECDSA signing: the key and the noise Z are secret. Allowed: the four ensure verdicts (key range, nonce
# candidate range, result not at infinity, r and s non-zero) and reports whose stack passes through
# ecdsa::output, which encodes the published r and s. Nothing else.
for m in ecdsa-sign256 ecdsa-sign384; do
  echo "taint($m): $(count $m) report(s); 4 must be verdicts, the rest inside ecdsa::output"
  python3 -I scripts/verdict_sites.py "$BIN" "$OUT/$m" 4 --output ecdsa::output >/dev/null || { python3 -I scripts/verdict_sites.py "$BIN" "$OUT/$m" 4 --output ecdsa::output | grep FAIL; fail=1; }
done
n=$(count ecdsa-control); echo "taint(ecdsa-control): $n report(s), the planted branch must be caught and refused"
report ecdsa-control | grep -q 'rusty_pk_taint::main' || { echo "taint(ecdsa-control): the planted branch was not reported in main"; fail=1; }
if python3 -I scripts/verdict_sites.py "$BIN" "$OUT/ecdsa-control" --output ecdsa::output >/dev/null; then echo "taint(ecdsa-control): verdict_sites accepted the planted branch"; fail=1; fi
# PKCS#8: only the private-key bytes inside the document are secret; structure is public. The only
# allowed reports are ensure verdicts: ECDSA scalar valid, public key not at infinity, and derived
# public key equal to the encoded one (3); Ed25519 the last only (1).
for m in pkcs8-ecdsa256 pkcs8-ed25519; do
  case $m in pkcs8-ecdsa256) want=3 ;; *) want=1 ;; esac
  n=$(count $m); echo "taint($m): $n error(s), want exactly $want (verdict branches only)"; [ "$n" -eq $want ] || fail=1
  python3 -I scripts/verdict_sites.py "$BIN" "$OUT/$m" $want || fail=1
done
python3 -I ../rusty_ct_check/scripts/disasm_audit.py "$BIN" scripts/disasm_limits.txt || fail=1
exit $fail
