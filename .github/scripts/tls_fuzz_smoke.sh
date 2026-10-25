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
# runner does not have. `FUZZ_FEATURES`, when set, is passed to `run` as
# cargo arguments (the long run uses `--no-default-features --features native`).
# `FUZZ_TARGETS`, when set, runs only those targets; each must be listed.
# Each target's full output streams to `<FUZZ_LOG_DIR>/<target>.log` as it
# runs (a temporary directory when unset), so an interrupted long run still
# leaves its log; only the last 25 lines are echoed.
# Deliberately no `set -e`: one failing
# target must not hide the others.
set -uo pipefail

secs="${1:?usage: tls_fuzz_smoke.sh <seconds-per-target> [min-targets]}"
min="${2:-8}"
# The command is built as an array. `FUZZ_TOOLCHAIN` (a dated nightly in CI)
# selects the toolchain with `+name`; unset, the active toolchain is used, so
# the script never silently asks rustup for a floating `nightly`. `CARGO_FUZZ`
# replaces the whole command (tests use a fake).
if [ -n "${CARGO_FUZZ:-}" ]; then
  # shellcheck disable=SC2206 # deliberate word splitting of a command
  fuzz=($CARGO_FUZZ)
else
  fuzz=(cargo)
  [ -n "${FUZZ_TOOLCHAIN:-}" ] && fuzz+=("+$FUZZ_TOOLCHAIN")
  fuzz+=(fuzz)
fi
# Resolve the log directory against the caller's directory, before the `cd`.
logs="${FUZZ_LOG_DIR:-$(mktemp -d)}"
if ! mkdir -p "$logs" || ! logs="$(cd "$logs" && pwd)"; then
  echo "::error::cannot create log directory $logs"; exit 1
fi
root="${FUZZ_ROOT:-$(dirname "$0")/../../crates/libs/net/rusty_tls}"
cd "$root" || { echo "::error::no fuzz root at $root"; exit 1; }

targets="$("${fuzz[@]}" list)" || { echo "::error::could not list fuzz targets"; exit 1; }
count="$(printf '%s\n' "$targets" | grep -c .)"
if [ "$count" -lt "$min" ]; then
  echo "::error::found $count fuzz targets, expected at least $min"
  exit 1
fi

if [ -n "${FUZZ_TARGETS:-}" ]; then
  for t in $FUZZ_TARGETS; do
    printf '%s\n' "$targets" | grep -qxF "$t" || { echo "::error::no fuzz target named $t"; exit 1; }
  done
  targets="$FUZZ_TARGETS"
  count="$(wc -w <<<"$targets")"
fi
# shellcheck disable=SC2206 # deliberate word splitting of cargo arguments
features=(${FUZZ_FEATURES:-})

failed=0
for t in $targets; do
  log="$logs/$t.log"
  "${fuzz[@]}" run "${features[@]}" "$t" ${FUZZ_TARGET_TRIPLE:+--target "$FUZZ_TARGET_TRIPLE"} -- "-max_total_time=$secs" >"$log" 2>&1
  status=$?
  tail -n 25 "$log"
  if [ "$status" -ne 0 ]; then
    echo "::error::fuzz target $t failed (exit $status)"
    failed=$((failed + 1))
  elif ! grep -qE 'Done [1-9][0-9]* runs' "$log"; then
    echo "::error::fuzz target $t ran nothing"
    failed=$((failed + 1))
  else
    echo "fuzz target $t: ok"
  fi
done

echo "fuzz smoke: $count targets, $failed failed"
[ "$failed" -eq 0 ]
