#!/usr/bin/env bash
# Run the rusty_tls certificate differential against the x509-limbo corpus.
#
#   limbo_differential.sh <command> [args...]
#
# Downloads limbo.json, checks it against a pinned sha256, and runs the command
# with LIMBO_JSON pointing at it. The command must report one passing test.
#
# The corpus lives upstream and changes, so a download failure or a hash that no
# longer matches is a *skip with a visible warning*, not a failure: an upstream
# edit must not break unrelated pull requests. It is also why this is not
# silent: the warning goes to the log and the step summary, and the pin is
# re-set by hand (see docs/research/TLS-EVIDENCE-BAR.md). When the corpus does
# match, any change in the results fails the run (tests/data/limbo_divergences.txt).
#
# Overrides (tests use them): LIMBO_URL, LIMBO_SHA256, LIMBO_DEST.
set -uo pipefail

url="${LIMBO_URL:-https://raw.githubusercontent.com/C2SP/x509-limbo/main/limbo.json}"
want="${LIMBO_SHA256:-611e337b9fb477b927bae65434650692cb9d2070ff5b05f975d0403510acd4de}"
dest="${LIMBO_DEST:-$(mktemp -d)/limbo.json}"
summary="${GITHUB_STEP_SUMMARY:-/dev/null}"

skip() {
  echo "::warning::x509-limbo differential SKIPPED: $1"
  printf '### x509-limbo differential\n**SKIPPED:** %s\n' "$1" >> "$summary"
  exit 0
}

curl -fsSL --retry 3 --max-time 120 -o "$dest" "$url" || skip "could not download $url"
got="$(sha256sum "$dest" | cut -d' ' -f1)"
[ "$got" = "$want" ] || skip "corpus sha256 is $got, pinned $want; re-pin after reviewing the new results"

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
