#!/usr/bin/env bash
# Run the rusty_tls live-internet check and report it, whatever happens.
#
#   live_internet_check.sh <command> [args...]
#
# The command's output (stdout and stderr) is always printed and written to the
# step summary, including when the command fails: this check is non-blocking in
# CI (continue-on-error), so the output is the only thing it leaves behind. The
# script exits non-zero if the command failed, or if it exited 0 without the one
# passing test (a zero-test run also exits 0).
#
# Deliberately no `set -e`: a failing command must not stop the report.
set -uo pipefail

summary="${GITHUB_STEP_SUMMARY:-/dev/null}"

status=0
out="$("$@" 2>&1)" || status=$?

printf '%s\n' "$out"

{
  echo '### rusty_tls native engine, live internet'
  echo '```'
  printf '%s\n' "$out" | grep -E '^(accounts|oauth2|www)[a-z0-9.]*:|test result|error|panicked' || true
  echo '```'
} >> "$summary"

if [ "$status" -ne 0 ]; then
  echo "::warning::the live internet check failed (exit $status); see the step output"
  exit 1
fi

if ! printf '%s\n' "$out" | grep -qE 'test result: ok\. 1 passed'; then
  echo "::warning::the live internet check did not report 1 passed; see the step output"
  exit 1
fi
