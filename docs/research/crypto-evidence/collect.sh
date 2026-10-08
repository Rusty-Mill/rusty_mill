#!/bin/sh
# Collects the evidence record for the native crypto crates: environment,
# pinned inputs, executed case counts, test results, constant-time checks and
# repeated timing runs, with raw numbers. Output goes to stdout; commit it as
# EVIDENCE-<date>.txt together with the commit it was produced at.
# usage: docs/research/crypto-evidence/collect.sh > docs/research/crypto-evidence/EVIDENCE-YYYY-MM-DD.txt
# Takes several minutes (timing tests run REPS times).
set -u
cd "$(git rev-parse --show-toplevel)"
REPS=${REPS:-5}
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
  head -3 "$d/tests/vectors/MANIFEST.txt" 2>/dev/null | sed 's/^/   /'
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
cargo test --release -p rusty_ct_check -p rusty_sha2 -p rusty_pk -p rusty_aead -p rusty_crypto_key 2>&1 \
  | sed 's/\x1b\[[0-9;]*m//g' | grep -E '^\s+Running|^   Doc-tests|^test result' | paste -sd' ' | sed 's/ Running /\nRunning /g; s/ Doc-tests /\nDoc-tests /g'
echo
echo "ignored tests (run below): $(grep -rn '#\[ignore' crates/foundation/rusty_ct_check crates/foundation/rusty_sha2 crates/foundation/rusty_pk crates/foundation/rusty_aead --include=*.rs | wc -l)"

section "lint and format"
cargo fmt --all --check >/dev/null 2>&1 && echo "fmt: clean" || echo "fmt: DIFFERENCES"
for c in rusty_ct_check rusty_sha2 rusty_pk rusty_aead rusty_crypto_key; do
  cargo clippy -p $c --all-targets -- -D warnings >/dev/null 2>&1 && echo "clippy $c: clean" || echo "clippy $c: FAILED"
done

section "constant-time checks (taint, disassembly budget, planted-leak controls)"
for c in rusty_ct_check rusty_sha2 rusty_aead rusty_pk; do
  s=crates/foundation/$c/scripts/valgrind_selftest.sh; [ "$c" = rusty_ct_check ] || s=crates/foundation/$c/scripts/ct_check.sh
  echo "-- $s"
  sh "$s" 2>&1 | grep -vE 'profiles for|^package:|^workspace:|Compiling|Finished|^warning' | sed 's/^/   /'
  echo "   exit=$?"
done

section "timing tests, $REPS repetitions each (|t|; threshold 4.5; ignored tests, run with --ignored)"
echo "classes and sample counts are in the test sources: rusty_sha2/tests/timing.rs (200000 samples), rusty_aead/tests/timing.rs (100000), rusty_pk/tests/timing.rs (20000); slowest 5% cropped; seeds fixed"
for c in rusty_sha2 rusty_aead rusty_pk; do
  i=1
  while [ $i -le "$REPS" ]; do
    cargo test --release -p $c --test timing -- --ignored --nocapture 2>&1 | grep -E '\|t\|' | sed "s/^/$c run $i: /"
    i=$((i+1))
  done
done

section "speed, one run each (not benchmarks)"
cargo test --release -p rusty_pk --test perf -- --ignored --nocapture 2>&1 | grep -E 'ours'
cargo test --release -p rusty_aead --test perf -- --ignored --nocapture 2>&1 | grep -E 'MB/s'
echo
echo "end of record"
