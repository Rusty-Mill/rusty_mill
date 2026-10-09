#!/bin/sh
# Runs the ignored `planted_early_exit_is_detected` timing test and fails unless
# exactly one test ran and passed. The test is #[ignore]d in plain `cargo test`
# (timing-sensitive), so collect.sh runs it through here. `CARGO` overrides the
# cargo binary (the tests inject a fake one).
set -u
CARGO=${CARGO:-cargo}
if out=$($CARGO test --release -p rusty_ct_check --lib -- --ignored --exact timing::tests::planted_early_exit_is_detected 2>&1); then
  status=0
else
  status=$?
fi
echo "$out" | grep -E '^test |test result' | sed 's/^/   /'
if [ "$status" -ne 0 ]; then
  echo "planted timing leak: NOT detected or the test failed to run (exit $status)"
  echo "$out" | tail -15 | sed 's/^/   /'
  exit 1
fi
if ! echo "$out" | grep -qE 'test result: ok\. 1 passed'; then
  echo "planted timing leak: the filter did not run exactly one passing test"
  exit 1
fi
echo "planted timing leak: detected (rusty_ct_check timing::tests::planted_early_exit_is_detected passed)"
