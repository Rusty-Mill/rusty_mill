#!/usr/bin/env bash
#
# Calibrate the decision-discipline rubric end-to-end, against both ground truths.
#
# Two stages, in order:
#   1. `calibrate`  — refit every metric curve to the corpus distribution and fit
#      within-role weights to **rank** (Spearman), writing assets/corpus/
#      fitted_config.json. Prints the cross-validated composite-vs-rank ρ.
#   2. `reconcile --promote` — load that config + the value model, compute each
#      metric's correlation with per-touch **ΔV**, and graduate any experimental
#      candidate metric (facing-ball, reverse-driving, last-defender, …) whose ΔV
#      signal clears --min-rho and points the right way: its `experimental` flag is
#      cleared and its weight set to ρ², written back to fitted_config.json.
#
# So the rubric is calibrated to cross-match skill (rank) *and* its new candidate
# metrics enter the composite only on in-match-impact evidence (ΔV).
#
# Needs the ranked-2v2 corpus `.replay` files on disk (gitignored, ~230 MB; fetched
# if BC_TOKEN is set) and a current assets/corpus/value_model.json (regenerate with
# scripts/retrain_value_model.sh after a value-feature change).
#
# Usage:
#   BC_TOKEN=<token> scripts/calibrate_scoring.sh          # fetch (if needed) + calibrate + promote
#   scripts/calibrate_scoring.sh                           # use already-present replays
#   scripts/calibrate_scoring.sh --min-rho 0.15            # stricter promotion threshold
#   scripts/calibrate_scoring.sh path/to/manifest.json     # alternate corpus
set -euo pipefail

cd "$(dirname "$0")/.."

MANIFEST="assets/corpus/manifest.json"
MIN_RHO=0.10
for a in "$@"; do
  case "$a" in
    --min-rho) shift; MIN_RHO="${1:?--min-rho needs a value}"; shift || true ;;
    --min-rho=*) MIN_RHO="${a#*=}" ;;
    *.json) MANIFEST="$a" ;;
  esac
done
CORPUS_DIR="$(dirname "$MANIFEST")"
MIN_REPLAYS=10

present=$(find "$CORPUS_DIR" -maxdepth 1 -name '*.replay' 2>/dev/null | wc -l | tr -d ' ')
echo "corpus replays present: $present (in $CORPUS_DIR)"
if [ "$present" -lt "$MIN_REPLAYS" ]; then
  if [ -n "${BC_TOKEN:-}" ]; then
    echo "Fetching corpus replays from the ballchasing API (gitignored)..."
    python "$CORPUS_DIR/refresh_corpus_replays.py"
  else
    echo "ERROR: only $present corpus replay(s) on disk and BC_TOKEN is unset." >&2
    echo "  BC_TOKEN=<token> python $CORPUS_DIR/refresh_corpus_replays.py" >&2
    exit 1
  fi
fi

if [ ! -f "$CORPUS_DIR/value_model.json" ]; then
  echo "ERROR: $CORPUS_DIR/value_model.json missing — promotion needs the value model." >&2
  echo "  run scripts/retrain_value_model.sh first" >&2
  exit 1
fi

echo
echo "[1/2] calibrate: refit curves + fit weights to rank -> fitted_config.json"
cargo run --release -q -p replay-scoring --bin calibrate -- "$MANIFEST"

echo
echo "[2/2] reconcile --promote: graduate ΔV-vindicated candidates (min-rho $MIN_RHO)"
cargo run --release -q -p replay-scoring --bin reconcile -- "$MANIFEST" --promote --min-rho "$MIN_RHO"

echo
echo "Done. Review the promotion verdicts above, then commit the fitted config:"
echo "  git add $CORPUS_DIR/fitted_config.json"
echo "  git commit -m 'scoring: recalibrate rubric + promote ΔV-vindicated candidates'"
