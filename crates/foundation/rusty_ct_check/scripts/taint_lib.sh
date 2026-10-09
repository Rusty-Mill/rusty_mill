# Sourced (`. ../rusty_ct_check/scripts/taint_lib.sh`) by the ct_check.sh scripts
# and valgrind_selftest.sh. Needs `BIN` (the example) and `fail=0` set, and
# `set -eu` in the caller.
#
# Every mode runs once, its output goes to a file, and the example's own exit
# status is checked. A pipeline ending in `|| true` would read "the example
# panicked / was misspelled / valgrind is missing" as "0 errors", which is the
# expected answer for the clean modes (Codex round 3 on #540).
OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT

# run_mode MODE: valgrind -q BIN MODE. A non-zero exit fails the check.
run_mode() {
  st=0
  valgrind -q "$BIN" "$1" >"$OUT/$1" 2>&1 || st=$?
  if [ "$st" -ne 0 ]; then
    echo "taint($1): valgrind/example exited $st, so its report does not count:"
    head -5 "$OUT/$1" | sed 's/^/   /'
    fail=1
  fi
}
# report MODE: the diagnostics (and the line after each) of a finished run.
report() { grep -E '^==[0-9]+== (Conditional|Use of)' -A1 "$OUT/$1" || true; }
# count MODE: how many diagnostics.
count() { grep -cE '^==[0-9]+== (Conditional|Use of)' "$OUT/$1" || true; }
