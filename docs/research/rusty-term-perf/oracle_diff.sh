#!/bin/sh
# Compare every fidelity golden (rusty_term_core) with libghostty-vt's grid for
# the same recording. Comparison oracle only: never run in CI, never linked
# into rusty_term. Needs network on first build (libghostty-vt prebuilt).
#   sh docs/research/rusty-term-perf/oracle_diff.sh [--diff]
set -eu
here=$(cd "$(dirname "$0")" && pwd)
corpus="$here/../../../crates/libs/ui/rusty_term/core/tests/fidelity/corpus"
out=${TMPDIR:-/tmp}/rusty-term-oracle
mkdir -p "$out"
(cd "$here/ghbench" && CARGO_TARGET_DIR="$out/target" cargo build --release -q)
for vt in "$corpus"/*.vt; do
    name=$(basename "$vt" .vt)
    "$out/target/release/ghbench" grid "$vt" > "$out/$name.grid"
    if cmp -s "$corpus/$name.grid" "$out/$name.grid"; then
        echo "same     $name"
    else
        echo "DIFFERS  $name  ($(diff "$corpus/$name.grid" "$out/$name.grid" | grep -c '^[<>]') lines)"
        if [ "${1:-}" = "--diff" ]; then diff "$corpus/$name.grid" "$out/$name.grid" || true; fi
    fi
done
