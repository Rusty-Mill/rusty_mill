#!/bin/bash
# #1166 smoke RERUN on BaileyAi: LOCAL-ONLY, no paid call. A fresh task, the inbox fixture (D146), lens run, read-back.
#   LLAMA_SWAP=<llama-swap-host>:<port> bash docs/projects/let-em-loose/1166-smoke-rerun.sh          # task smoke-2; TASK=smoke-3 for another
#
# It must run binaries built from the fixes (rusty_orch: no `executed` evidence from a lens, D144; no panic on a closed
# stdout), so run this FIRST whenever those PRs have just merged or you have not deployed since:
#   git switch main && git pull --ff-only && bash docs/projects/let-em-loose/1092-deploy.sh
# This script refuses to start if the installed rusty_orch lacks either fix. It then runs 1166-smoke-run.sh (stage,
# lens check, open, assign, seed Q and A, inbox fixture A2, detached lens run with the 90-minute box, read-back).
# Interrupted? TASK=<id> LLAMA_SWAP=<llama-swap-host>:<port> bash docs/projects/let-em-loose/1166-smoke-resume.sh waits for the run and reads back.
set -euo pipefail

export TASK=${TASK:-smoke-2}
BIN=${LEL_BIN:-/opt/lel/current/bin}   # LEL_BIN and GUARD_ONLY=1 exist to test the guards
HERE=$(cd "$(dirname "$0")" && pwd)

[ -x "$BIN/rusty_orch" ] || { echo "STOP: $BIN/rusty_orch is missing: run 1092-deploy.sh" >&2; exit 1; }
[ "$TASK" != smoke-1 ] || { echo "STOP: smoke-1 is run 1's task; use a new id" >&2; exit 1; }

# Fix 1 (D144): the reply protocol text no longer offers `executed`.
if grep -aqF '"evidence_kind": "executed"' "$BIN/rusty_orch"; then
  echo "STOP: the installed rusty_orch still offers executed evidence to lenses (D144 is not in this build)." >&2
  echo "      git switch main && git pull --ff-only && bash docs/projects/let-em-loose/1092-deploy.sh" >&2
  exit 1
fi
# Fix 2: a closed stdout must not panic the lens subcommands.
# (`>&-` would not do: Rust ignores a closed descriptor; only a pipe whose reader is gone gives the EPIPE that panicked.)
LOG=$(mktemp); trap 'rm -f "$LOG"' EXIT
set +e
( sleep 0.3; exec "$BIN/rusty_orch" lens check --help 2>"$LOG" ) | true
CODE=${PIPESTATUS[0]}
set -e
if grep -q panicked "$LOG" || [ "$CODE" != 0 ]; then
  echo "STOP: the installed rusty_orch panics when stdout is closed (the closed-stdout fix is not in this build)." >&2
  echo "      git switch main && git pull --ff-only && bash docs/projects/let-em-loose/1092-deploy.sh" >&2
  exit 1
fi
echo "installed rusty_orch has both fixes ($(readlink /opt/lel/current 2>/dev/null || true)); task $TASK"

[ -z "${GUARD_ONLY:-}" ] || { echo "guards ok (GUARD_ONLY)"; exit 0; }
exec bash "$HERE/1166-smoke-run.sh"
