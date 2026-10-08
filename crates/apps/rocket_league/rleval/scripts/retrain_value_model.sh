#!/usr/bin/env bash
#
# Retrain the corpus value model end-to-end and report the held-out AUC.
#
# One command for the whole loop:
#   1. ensure the ranked-2v2 corpus `.replay` files are on disk (fetch if a
#      BC_TOKEN is set and they're missing — they're gitignored, ~230 MB);
#   2. run `train_corpus`, which parses the corpus, prints the train/val
#      log-loss and the **logistic-vs-GBT VAL_AUC head-to-head**, and writes the
#      shipped GBT to assets/corpus/value_model.json;
#   3. remind you to commit the refreshed (committed) model.
#
# Run this after changing the value feature set (e.g. the vcfg-v2 10->18 feature
# expansion) so the shipped corpus model matches the current featurizer. The
# per-match value path (the app/viewer ΔV) needs no retrain — it fits a fresh
# logistic per replay — but the CLI tools that load value_model.json
# (replay-skills, scoring reconcile) and any GBT-based analysis do.
#
# Usage:
#   BC_TOKEN=<token> scripts/retrain_value_model.sh            # fetch (if needed) + train
#   scripts/retrain_value_model.sh                            # train on already-present replays
#   scripts/retrain_value_model.sh path/to/manifest.json      # alternate corpus
#
# Get a token at https://ballchasing.com/upload (Account -> API key).
set -euo pipefail

cd "$(dirname "$0")/.."
MANIFEST="${1:-assets/corpus/manifest.json}"
CORPUS_DIR="$(dirname "$MANIFEST")"
MIN_REPLAYS=10

present=$(find "$CORPUS_DIR" -maxdepth 1 -name '*.replay' 2>/dev/null | wc -l | tr -d ' ')
echo "corpus replays present: $present (in $CORPUS_DIR)"

if [ "$present" -lt "$MIN_REPLAYS" ]; then
  if [ -n "${BC_TOKEN:-}" ]; then
    echo "Fetching corpus replays from the ballchasing API (gitignored, not committed)..."
    python "$CORPUS_DIR/refresh_corpus_replays.py"
  else
    echo "ERROR: only $present corpus replay(s) on disk and BC_TOKEN is unset." >&2
    echo "Set a ballchasing token and re-run, or fetch manually:" >&2
    echo "  BC_TOKEN=<token> python $CORPUS_DIR/refresh_corpus_replays.py" >&2
    exit 1
  fi
fi

echo "Training the value model on the corpus (release build; parses every replay)..."
cargo run --release -q -p replay-value --bin train_corpus -- "$MANIFEST"

echo
echo "Done. Review the VAL_AUC head-to-head above (the GBT is the shipped model)."
echo "If it looks good, commit the refreshed model:"
echo "  git add $CORPUS_DIR/value_model.json"
echo "  git commit -m 'value: retrain corpus model on vcfg-v2 feature set'"
