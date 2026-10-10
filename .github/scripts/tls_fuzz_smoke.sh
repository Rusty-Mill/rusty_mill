#!/usr/bin/env bash
# Run every rusty_tls fuzz target for a fixed time and fail on any finding.
#
#   tls_fuzz_smoke.sh <seconds-per-target> [min-targets]
#
# A crash makes libFuzzer exit non-zero, but so would a build failure, and a
# target that starts and executes nothing exits 0. So each target must also
# report `Done N runs` with N > 0, and fewer targets than `min-targets` is an
# error (a renamed directory would otherwise fuzz nothing and pass).
#
# `CARGO_FUZZ` overrides the tool (tests use a fake); `FUZZ_ROOT` is the
# directory holding the fuzz package; `FUZZ_TARGET_TRIPLE`, when set, is passed
# as `--target`. cargo-fuzz otherwise builds for the target it was itself
# compiled for, and the prebuilt binary CI installs is musl, whose std the
# runner does not have. Deliberately no `set -e`: one failing
# target must not hide the others.
set -uo pipefail

secs="${1:?usage: tls_fuzz_smoke.sh <seconds-per-target> [min-targets]}"
min="${2:-8}"
fuzz="${CARGO_FUZZ:-cargo +nightly fuzz}"
root="${FUZZ_ROOT:-$(dirname "$0")/../../crates/libs/net/rusty_tls}"
cd "$root" || { echo "::error::no fuzz root at $root"; exit 1; }

# shellcheck disable=SC2086 # $fuzz is a command plus arguments
targets="$($fuzz list)" || { echo "::error::could not list fuzz targets"; exit 1; }
count="$(printf '%s\n' "$targets" | grep -c .)"
if [ "$count" -lt "$min" ]; then
  echo "::error::found $count fuzz targets, expected at least $min"
  exit 1
fi

failed=0
for t in $targets; do
  # shellcheck disable=SC2086
  out="$($fuzz run "$t" ${FUZZ_TARGET_TRIPLE:+--target "$FUZZ_TARGET_TRIPLE"} -- "-max_total_time=$secs" 2>&1)"
  status=$?
  printf '%s\n' "$out" | tail -n 25
  if [ "$status" -ne 0 ]; then
    echo "::error::fuzz target $t failed (exit $status)"
    failed=$((failed + 1))
  elif ! printf '%s\n' "$out" | grep -qE 'Done [1-9][0-9]* runs'; then
    echo "::error::fuzz target $t ran nothing"
    failed=$((failed + 1))
  else
    echo "fuzz target $t: ok"
  fi
done

echo "fuzz smoke: $count targets, $failed failed"
[ "$failed" -eq 0 ]
