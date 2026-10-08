#!/bin/sh
# Collects the evidence record for the native crypto crates: environment,
# pinned inputs, executed case counts, test results, constant-time checks and
# repeated timing runs, with raw numbers. Output goes to stdout; commit it as
# EVIDENCE-<date>.txt together with the commit it was produced at.
# usage: docs/research/crypto-evidence/collect.sh > docs/research/crypto-evidence/EVIDENCE-YYYY-MM-DD.txt
# Takes several minutes (timing tests run REPS times).
set -u
FAIL=0   # set to 1 by any failed check; the script exits non-zero at the end
fail() { FAIL=1; echo "!! FAILED: $1"; }
cd "$(git rev-parse --show-toplevel)"
REPS=${REPS:-40}
section() { printf '\n== %s ==\n' "$1"; }

section "implementation"
echo "commit: $(git rev-parse HEAD)"
echo "branch: $(git rev-parse --abbrev-ref HEAD)"
echo "tree: $( [ -z "$(git status --porcelain -- crates/foundation docs/research/CRYPTO-REPLACEMENT-PLAN.md)" ] && echo clean || echo 'UNCOMMITTED CHANGES in crates/foundation or the plan')"
echo "date (UTC): $(date -u +%Y-%m-%dT%H:%M:%SZ)"

section "environment"
rustc -Vv
cargo -V
echo "cpu: $(grep -m1 'model name' /proc/cpuinfo | cut -d: -f2- | sed 's/^ //')"
echo "cores: $(nproc)"
echo "cpu flags of interest: $(grep -m1 -o -w -E 'aes|pclmulqdq|avx2|sha_ni' /proc/cpuinfo | sort -u | tr '\n' ' ')"
echo "kernel: $(uname -srm)"
echo "valgrind: $(valgrind --version)"
echo "profile: cargo test --release (opt-level 3, workspace default; no target-cpu flags; RUSTFLAGS='${RUSTFLAGS:-}')"
echo "virtualised: $(systemd-detect-virt 2>/dev/null || echo 'unknown (sandbox VM; timing is noisy)')"

section "dependencies pinned"
grep -A1 'name = "ring"' Cargo.lock | tr '\n' ' '; echo

section "vector files"
for d in crates/foundation/rusty_sha2 crates/foundation/rusty_pk crates/foundation/rusty_aead; do
  echo "-- $d/tests/vectors/MANIFEST.txt"
  head -2 "$d/tests/vectors/MANIFEST.txt" 2>/dev/null | sed 's/^/   /'
  (cd "$d/tests/vectors" && sha256sum *.json *.txt 2>/dev/null | grep -v MANIFEST | sed 's/^/   /')
done

section "vector case counts (declared vs parsed; executed = parsed unless noted)"
python3 -I - <<'PY'
import json, glob, collections
total = 0
for f in sorted(glob.glob("crates/foundation/*/tests/vectors/*.json")):
    d = json.load(open(f))
    parsed = sum(len(g["tests"]) for g in d["testGroups"])
    res = collections.Counter(t["result"] for g in d["testGroups"] for t in g["tests"])
    flag = "" if parsed == d.get("numberOfTests") else "  MISMATCH with header"
    print(f"{f.split('/')[2]:12} {f.split('/')[-1]:44} declared={d.get('numberOfTests')} parsed={parsed} {dict(res)}{flag}")
    total += parsed
print("total Wycheproof cases parsed:", total)
# documented non-uniform handling
ch = json.load(open("crates/foundation/rusty_aead/tests/vectors/chacha20_poly1305_test.json"))
wrong_nonce = sum(1 for g in ch["testGroups"] for t in g["tests"] if len(t["iv"]) != 24)
print("chacha20_poly1305: cases with a non-96-bit nonce, asserted invalid but not run through the AEAD (typed API cannot express them):", wrong_nonce)
pss = 0; native = 0
H = {"SHA-256": 32, "SHA-384": 48, "SHA-512": 64}
for f in glob.glob("crates/foundation/rusty_pk/tests/vectors/rsa_pss_*.json"):
    d = json.load(open(f))
    for g in d["testGroups"]:
        for t in g["tests"]:
            pss += 1
            native += (g["sha"] == g["mgfSha"] and g["sLen"] == H.get(g["sha"], -1))
print(f"rsa_pss: {pss} cases across files; {native} use parameters in our profile (hash = mgf hash, salt = hash length); the rest are checked only for equality with ring and, if valid under other parameters, rejection")
PY
for f in crates/foundation/rusty_pk/tests/vectors/ecdsa_p256_sha384.txt crates/foundation/rusty_pk/tests/vectors/rsa_forgeries.txt; do echo "generated: $f lines=$(wc -l < $f)"; done

section "tests (executed / failed / ignored per binary)"
cargo test --release -p rusty_ct_check -p rusty_sha2 -p rusty_pk -p rusty_aead -p rusty_crypto_key >/dev/null 2>&1 || fail "cargo test"
cargo test --release -p rusty_ct_check -p rusty_sha2 -p rusty_pk -p rusty_aead -p rusty_crypto_key 2>&1 \
  | sed 's/\x1b\[[0-9;]*m//g' | grep -E '^\s+Running|^   Doc-tests|^test result' | paste -sd' ' | sed 's/ Running /\nRunning /g; s/ Doc-tests /\nDoc-tests /g'
echo
echo "ignored tests (run below): $(grep -rn '#\[ignore' crates/foundation/rusty_ct_check crates/foundation/rusty_sha2 crates/foundation/rusty_pk crates/foundation/rusty_aead --include=*.rs | wc -l)"

section "lint and format"
cargo fmt --all --check >/dev/null 2>&1 && echo "fmt: clean" || { echo "fmt: DIFFERENCES"; fail fmt; }
# Clippy lints depend on the compiler AND on the declared rust-version (some lints only fire above a
# minimum version), so lint on CI's pinned toolchain as well as the local default: a clean run on one
# compiler has missed a failure on the other (chunks_exact_to_as_chunks, 1.98.1, PR #540).
CI_TC_LINT=$(sed -n 's/^  RUST_TOOLCHAIN: "\(.*\)"/\1/p' .github/workflows/ci.yml | head -1)
for tc in "$CI_TC_LINT" stable; do
  for c in rusty_ct_check rusty_sha2 rusty_pk rusty_aead rusty_crypto_key; do
    RUSTUP_TOOLCHAIN=$tc cargo clippy -p $c --all-targets -- -D warnings >/dev/null 2>&1 && echo "clippy $c ($tc): clean" || { echo "clippy $c ($tc): FAILED"; fail "clippy $c on $tc"; }
  done
done

section "constant-time checks (taint, disassembly budget, planted-leak controls), per compiler"
# The budgets and the compiler's treatment of masked selects are specific to a compiler version, so the
# checks run on every toolchain named here; the default includes the version CI builds with
# (RUST_TOOLCHAIN in .github/workflows/ci.yml) as well as the local default.
CI_TC=$(sed -n 's/^  RUST_TOOLCHAIN: "\(.*\)"/\1/p' .github/workflows/ci.yml | head -1)
CT_TOOLCHAINS=${CT_TOOLCHAINS:-"$CI_TC stable"}
for tc in $(echo $CT_TOOLCHAINS | tr ' ' '\n' | sort -u); do
  if ! RUSTUP_TOOLCHAIN=$tc rustc -V >/dev/null 2>&1; then echo "toolchain $tc: NOT INSTALLED"; fail "toolchain $tc missing"; continue; fi
  echo "-- toolchain $tc ($(RUSTUP_TOOLCHAIN=$tc rustc -V))"
  for c in rusty_ct_check rusty_sha2 rusty_aead rusty_pk; do
    sc=crates/foundation/$c/scripts/valgrind_selftest.sh; [ "$c" = rusty_ct_check ] || sc=crates/foundation/$c/scripts/ct_check.sh
    out=$(RUSTUP_TOOLCHAIN=$tc sh "$sc" 2>&1); st=$?
    echo "$out" | grep -vE 'profiles for|^package:|^workspace:|Compiling|Finished|^warning' | sed 's/^/   /'
    echo "   [$c] exit=$st"
    [ $st -eq 0 ] || fail "ct check $c on $tc"
  done
done

section "timing tests: $REPS repetitions per test, with A/A (no-leak) baselines"
cat <<'TXT'
How to read this: each number is |t| from one run (dudect-style Welch t, slowest 5% cropped, fixed
seed, ignored tests run with --ignored). Threshold 4.5. The A/A tests do identical work in both
classes, so their spread is this machine's noise: a leak claim needs a clear, repeatable gap above
the A/A spread, and a pass is only "no leak detected under these conditions". Samples per run:
rusty_sha2 200000, rusty_aead 100000, rusty_pk 20000. Classes are written in each test's comments.
The tests time only the operation; class-dependent setup is outside the clock and writes into one
buffer shared by both classes. An earlier version that set up inside the timed region reported
|t| up to 25 on this same code (see "harness history" in the plan, stage 3/4 notes).
TXT
BASE_NONZERO=0
summ() { sort -n | awk '{a[NR]=$1} END{c=0; for(i=1;i<=NR;i++) if(a[i]>4.5) c++; printf "n=%d min=%s median=%s p90=%s max=%s above_4.5=%d", NR, a[1], a[int((NR+1)/2)], a[int(NR*0.9)], a[NR], c}'; }
series() { # crate test-name label
  c=$1; t=$2; l=$3
  bin=""
  for b in $(ls -t target/release/deps/timing-* 2>/dev/null | grep -v '\.d$'); do
    "$b" --ignored --list 2>/dev/null | grep -q "^$t: test" && { bin=$b; break; }
  done
  [ -n "$bin" ] || { echo "$l: test $t not found"; return; }
  raw=""; i=1; nonzero=0; unparsed=0
  while [ $i -le "$REPS" ]; do
    # Capture the test binary's own exit status: it is non-zero exactly when |t| >= 4.5.
    out=$("$bin" --ignored --nocapture "$t" 2>&1); st=$?
    v=$(echo "$out" | grep -E '\|t\| = ' | grep -v panicked | head -1 | sed 's/.*|t| = //')
    [ -n "$v" ] || unparsed=$((unparsed+1))
    [ $st -eq 0 ] || nonzero=$((nonzero+1))
    raw="$raw $v"; i=$((i+1))
  done
  echo "$l: $(echo $raw | tr ' ' '\n' | summ); test-binary exits non-zero: $nonzero of $REPS"
  echo "   raw:$raw"
  [ $unparsed -eq 0 ] || fail "$l: $unparsed run(s) produced no |t| (test did not run)"
  case "$l" in
    A/A*) BASE_NONZERO=$nonzero ;;   # this machine's false-alarm count for the crate that follows
    *) # A leak test fails the record if it alarms clearly more often than the no-leak baseline.
       limit=$((2 * BASE_NONZERO + 1))
       [ $nonzero -le $limit ] || fail "$l: $nonzero alarms of $REPS exceeds the baseline allowance ($limit; baseline $BASE_NONZERO)" ;;
  esac
}
for c in rusty_sha2 rusty_aead rusty_pk; do cargo test --release -p $c --test timing --no-run >/dev/null 2>&1; done
series rusty_sha2 null_calibration_identical_classes "A/A  HMAC-SHA512 (baseline)"
series rusty_sha2 hmac_sha512_key_classes_not_distinguishable "HMAC-SHA512 fixed vs other key"
series rusty_sha2 hmac_sha256_key_classes_not_distinguishable "HMAC-SHA256 fixed vs other key"
series rusty_aead null_calibration_identical_classes "A/A  ChaCha20-Poly1305 seal (baseline)"
series rusty_aead key_classes_not_distinguishable "seal: fixed vs other key"
series rusty_aead plaintext_classes_not_distinguishable "seal: all-zero vs all-ones plaintext"
series rusty_aead tag_mismatch_position_not_distinguishable "open: first-byte vs last-byte tag mismatch"
series rusty_pk null_calibration_identical_classes "A/A  X25519 (baseline)"
series rusty_pk scalar_classes_not_distinguishable "x25519: sparse vs dense scalar"
series rusty_pk fixed_vs_random_scalar "x25519: fixed vs other scalar"

section "speed, one run each (not benchmarks)"
cargo test --release -p rusty_pk --test perf -- --ignored --nocapture 2>&1 | grep -E 'ours'
cargo test --release -p rusty_aead --test perf -- --ignored --nocapture 2>&1 | grep -E 'MB/s'
echo
if [ $FAIL -eq 0 ]; then echo "RESULT: all checks passed (this is evidence, not proof)"; else echo "RESULT: FAILED, see the !! lines above"; fi
echo "end of record"
exit $FAIL
