#!/usr/bin/env bash
# Run the rusty_tls certificate differential against the x509-limbo corpus.
#
#   limbo_differential.sh <command> [args...]
#
# Downloads limbo.json, checks it against a pinned sha256, and runs the command
# with LIMBO_JSON pointing at it. The command must report one passing test.
#
# The corpus is fetched from an immutable upstream commit and checked against a
# pinned sha256, so the evidence cannot change under the test. If it cannot be
# fetched or does not match, the step FAILS: a differential that quietly stops
# running is worse than one that is red. Bumping the pin is a deliberate act
# (pick a new commit, run the test, review the new results, bless the golden,
# update both values below); see docs/research/TLS-EVIDENCE-BAR.md.
#
# Overrides (tests use them): LIMBO_URL, LIMBO_SHA256, LIMBO_DEST, LIMBO_RETRIES.
set -uo pipefail

url="${LIMBO_URL:-https://raw.githubusercontent.com/C2SP/x509-limbo/554528a9b0c0d95e071f55de018326f0b65a8364/limbo.json}"
want="${LIMBO_SHA256:-611e337b9fb477b927bae65434650692cb9d2070ff5b05f975d0403510acd4de}"
dest="${LIMBO_DEST:-$(mktemp -d)/limbo.json}"
summary="${GITHUB_STEP_SUMMARY:-/dev/null}"

fail() {
  echo "::error::x509-limbo differential could not run: $1"
  printf '### x509-limbo differential\n**FAILED TO RUN:** %s\n' "$1" >> "$summary"
  exit 1
}

curl -fsSL --retry "${LIMBO_RETRIES:-5}" --retry-all-errors --max-time 180 -o "$dest" "$url" || fail "could not download $url"
got="$(sha256sum "$dest" | cut -d' ' -f1)"
[ "$got" = "$want" ] || fail "corpus sha256 is $got, pinned $want"

out="$(LIMBO_JSON="$dest" "$@" 2>&1)"
status=$?
printf '%s\n' "$out"
{
  echo '### x509-limbo differential'
  echo '```'
  printf '%s\n' "$out" | grep -E '^limbo cases|skipped|expect|fail |ok  |test result|looser|stricter|panicked|^error' || true
  echo '```'
} >> "$summary"

if [ "$status" -ne 0 ]; then
  echo "::error::the x509-limbo differential failed (exit $status)"
  exit 1
fi
if ! printf '%s\n' "$out" | grep -qE 'test result: ok\. 1 passed'; then
  echo "::error::the x509-limbo differential did not report 1 passed -- is the test still #[ignore]d and named?"
  exit 1
fi
