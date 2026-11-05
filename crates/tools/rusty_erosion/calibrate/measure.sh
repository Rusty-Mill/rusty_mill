#!/bin/sh
# measure.sh EROSION_BIN REPO OUT NUMBER BRANCH SHA: one merged PR's erosion line, its merge
# commit measured against its first parent in a throwaway worktree, written to OUT/NUMBER.tsv as
# "NUMBER<TAB>BRANCH<TAB>SHA<TAB><line>". EROSION_BIN must be an absolute path; OUT is created if missing.
# REPO needs full history (`git fetch --unshallow`). Skips a PR already measured, so a stopped run
# resumes. See docs/projects/let-em-loose/erosion-calibration.md for the full command.
set -eu
BIN=$1 REPO=$2 OUT=$3 N=$4 REF=$5 SHA=$6
case $BIN in /*) ;; *) echo "measure.sh: EROSION_BIN must be absolute: $BIN" >&2; exit 2 ;; esac
mkdir -p "$OUT"
[ -s "$OUT/$N.tsv" ] && exit 0
W=$(mktemp -d)
trap 'git -C "$REPO" worktree remove --force "$W" 2>/dev/null || true' EXIT
git -C "$REPO" worktree add -q --detach "$W" "$SHA"
BASE=$(git -C "$W" rev-parse "$SHA^1")
LINE=$(cd "$W" && "$BIN" check --base "$BASE" --report-only)
printf '%s\t%s\t%s\t%s\n' "$N" "$REF" "$SHA" "$LINE" > "$OUT/$N.tsv"
