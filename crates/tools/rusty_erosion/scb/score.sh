#!/usr/bin/env bash
# Scores every SlopCodeBench reference-solution checkpoint with rusty_erosion
# and with SCB's own scorer, one TSV row each (card #705 Phase A, D201).
#
#   score.sh <scb-problems checkout> > rows.tsv
#
# SCB_CHECK overrides the scorer command (default: the pinned version the
# SCB harness uses). Run from the repository root. No model calls.
set -euo pipefail

problems=${1:?usage: score.sh <scb-problems dir>}
scb_check=${SCB_CHECK:-uvx scb-check==0.1.3}
cargo build -q -p rusty_erosion --example measure_tree
measure=target/debug/examples/measure_tree
here=$(dirname "$0")

printf 'problem\tcheckpoint\tloc\ttotal_cc\tfunctions\tconc_num\tconc_den\tdup_num\tdup_den\tunmeasured\tscb_erosion\tscb_verbosity\tscb_clone_loc\tscb_total_loc\n'
for dir in "$problems"/*/solutions/checkpoint_*; do
  [ -d "$dir" ] || continue
  problem=$(basename "$(dirname "$(dirname "$dir")")")
  checkpoint=${dir##*_}
  ours=$("$measure" "$dir")
  # shellcheck disable=SC2086 # scb_check is a command line, split on purpose.
  theirs=$($scb_check check --report --include-all "$dir" | python3 -I "$here/summarize.py" scb)
  printf '%s\t%s\t%s\t%s\n' "$problem" "$checkpoint" "$ours" "$theirs"
done
