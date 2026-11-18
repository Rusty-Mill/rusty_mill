#!/bin/bash
# #1166 inbox fixture (D146): seed ONE more irreversible action claim, A2, with two fixture supports, so it reaches
# the strict accept line (12) and escalates 'irreversible' at seed time. That puts a real escalation in `bbp inbox`
# and the Mill Board inbox before any model is called, so the inbox mechanics are exercised even when the lenses
# drop A (as they did in run 1). It changes no threshold and no escalation rule: it only posts through the same
# `bbp mcp` seat a lens uses, as principals `seed` and `seed-s1`, `seed-s2` (declared in the toy roster).
# The posts say they are fixture support; A2's escalation is NOT a lens finding.
# A2's text says it is a proposed action that has not run (Nano, Q61, 2026-11-18). It used to read 'Run `sweep --yes` ... after
# the dry-run list was reviewed', which the lenses took for a test report and cited as a run (smoke-3, smoke-4, smoke-6), and
# which asserted a review that never happened. Seed claim A (in 1166-smoke-run.sh) keeps its original wording as the control.
#
# Usage (called by 1166-smoke-run.sh; also runnable alone):
#   1166-seed-inbox.sh DIR TASK BRIEF_ART_ID [BIN]
# R (default: sudo -H -u orch-runner) is the prefix for every bbp call; R= runs it as the current user (tests).
set -euo pipefail
unset CDPATH   # a set CDPATH makes `cd` print the directory

D=${1:?usage: 1166-seed-inbox.sh DIR TASK BRIEF_ART_ID [BIN]}
TASK=${2:?task}
BRIEF=${3:?brief artifact id}
BIN=${4:-/opt/lel/current/bin}
R=${R-sudo -H -u orch-runner}
[[ $BRIEF =~ ^[0-9]+$ ]] || { echo "STOP: brief id is not a number: '$BRIEF'" >&2; exit 1; }
cd /

# One post through a fresh `bbp mcp` session as PRINCIPAL; prints the posted message id.
post_as() {
  local who=$1 args=$2 out
  out=$(printf '%s\n' \
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"fixture","version":"0"}}}' \
    '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
    "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"post\",\"arguments\":$args}}" \
    | $R env BBP_DIR="$D" BBP_TASK="$TASK" BBP_PRINCIPAL="$who" BBP_SESSION=yes "$BIN/bbp" mcp)
  echo "$out" >&2
  jq -er 'select(.id==2).result.content[0].text|fromjson|.Posted' <<<"$out"
}

A2=$(post_as seed '{"op":"fixture-a2","kind":"claim","text":"Proposed action, not run: run `sweep --yes` on the toy data directory with DAYS 7.","domains":["sweep"],"stakes":"medium","reversible":false,"deps":[],"refs":["art:'"$BRIEF"'"]}')
echo "A2=$A2"
for who in seed-s1 seed-s2; do
  post_as "$who" '{"op":"fixture-support-'"$who"'","kind":"evidence","claim":'"$A2"',"direction":"support","evidence_kind":"cited","strength":3,"note":"Fixture support (D146) so this irreversible claim reaches the strict line and escalates; not a lens finding.","evidence":["art:'"$BRIEF"'"],"refs":["art:'"$BRIEF"'"]}' >/dev/null
done

INBOX=$($R "$BIN/bbp" inbox --dir "$D" --task "$TASK" --json yes)
echo "$INBOX"
jq -e --argjson a "$A2" 'any(.[]; (tostring|test("\\b"+($a|tostring)+"\\b")) and (tostring|test("rreversible")))' <<<"$INBOX" >/dev/null \
  || { echo "STOP: A2 (msg $A2) is not in the inbox as an irreversible escalation: see the output above" >&2; exit 1; }
echo "A2 (msg $A2) escalated 'irreversible': it is in the inbox. Tap Reject (or drop) on it there."
