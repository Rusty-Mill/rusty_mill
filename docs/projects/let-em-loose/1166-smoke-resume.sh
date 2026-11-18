#!/bin/bash
# #1166 smoke run, RESUME after an interrupted step 6 (1166-smoke-run.sh). LOCAL-ONLY, no paid call.
#   LLAMA_SWAP=<llama-swap-host>:<port> bash docs/projects/let-em-loose/1166-smoke-resume.sh
# Safe to run at any time, and again after Ctrl-C:
#   1. if a `lens run` for this task is still alive (an interrupt that closed the terminal does not stop it), it waits
#      for that run to end and never starts a second one;
#   2. then it starts ONE more `lens run` on the same task, state and experiment, detached (setsid), so Ctrl-C or a
#      closed ssh session only stops this script's polling, not the run. `lens run` restores the meter and the
#      scheduler's memory from --state; a crash mid-round asks that round's jobs again and the replayed ops record
#      once; a board that has gone quiet starts no job and ends at once. task_calls 60 counts across both runs;
#   3. then it reads everything back.
# It does not re-seed, re-open or re-stage. Everything is logged to ~/smoke-1/resume-<time>.log.
set -euo pipefail
unset CDPATH   # a set CDPATH makes `cd` print the directory
HERE=$(cd -- "$(dirname -- "$0")" && pwd)   # before cd /, so a relative $0 still resolves

TASK=${TASK:-smoke-1}
OUT=$HOME/$TASK
LOG=$OUT/resume-$(date -u +%Y%m%dT%H%M%SZ).log
exec > >(tee -a "$LOG") 2>&1
echo "log: $LOG"

LLAMA_SWAP=${LLAMA_SWAP:?set to the llama-swap host:port}   # e.g. LLAMA_SWAP=<llama-swap-host>:<port>
cd /
R="sudo -H -u orch-runner"   # orch-runner cannot enter baileyrd's directories: every call below runs from /
BIN=/opt/lel/current/bin
D=/srv/lel/bbp
TOY=/srv/lel/toy
CFG=/srv/lel/lens-local.json
RUN_RE="rusty_orch lens run .*--task $TASK( |\$)"

die() { echo "STOP: $*" >&2; exit 1; }
trap 'echo; echo "Interrupted. A lens run already started keeps running on its own. Run this script again to wait for it and read back."; exit 130' INT TERM

alive() { pgrep -u orch-runner -f "$RUN_RE" >/dev/null; }
status() {
  local p; p=$(pgrep -u orch-runner -o -f "rusty_orch lens run .*--task $TASK" || true)
  echo "$(date -u +%T)Z lens run alive: pid ${p:-?} elapsed $(ps -o etime= -p "${p:-0}" 2>/dev/null | tr -d ' ' || true);" \
       "llama-swap loaded: $(curl -s -m 5 http://$LLAMA_SWAP/running | jq -r '[.running[].model]|join(",")' 2>/dev/null || echo '?')"
}
wait_for_end() {
  while alive; do status; sleep 60; done
  echo "$(date -u +%T)Z no lens run for $TASK is alive"
}

# ---- 0. Pre-flight
echo "=== 0. pre-flight $(date -u +%FT%TZ)"
command -v jq >/dev/null || die "jq is not installed"
[ -x "$BIN/bbp" ] && [ -x "$BIN/rusty_orch" ] || die "$BIN/{bbp,rusty_orch} missing"
$R $BIN/bbp card --dir $D --task "$TASK" >"$OUT/card-resume-start.json" || die "task $TASK is not open in $D: use 1166-smoke-run.sh"
cat "$OUT/card-resume-start.json"; echo
[ -s "$OUT/seed.out" ] || die "$OUT/seed.out missing: the task was not seeded by 1166-smoke-run.sh"
Q=$(jq -r 'select(.id==2).result.content[0].text|fromjson|.Posted' "$OUT/seed.out")
A=$(jq -r 'select(.id==3).result.content[0].text|fromjson|.Posted' "$OUT/seed.out")
[[ $Q =~ ^[0-9]+$ && $A =~ ^[0-9]+$ ]] || die "cannot read Q and A from $OUT/seed.out"
A2=$(cat "$OUT/a2.id" 2>/dev/null || true)
echo "Q=$Q A=$A A2=${A2:-none}"
echo "--- processes now"; pgrep -u orch-runner -af "rusty_orch|bbp mcp" | cut -c1-160 || echo "none"

# ---- 1. Let a surviving run finish; never start a second one beside it
echo "=== 1. wait for a surviving run"
if alive; then
  echo "A lens run for $TASK is still alive (probably the interrupted one: Ctrl-C closed the terminal, not the run)."
  echo "Its stdout record went to a closed pipe, so it is rebuilt below from 'lens record', 'bbp card' and 'bbp why'. Its 90-minute"
  echo "timeout still applies. Waiting (Ctrl-C only stops this wait)."
fi
wait_for_end

# ---- 2. Resume: one more run on the same task (detached; the exit code is written at the end of its stdout)
echo "=== 2. resume lens run $(date -u +%FT%TZ)"
STAMP=$(date -u +%Y%m%dT%H%M%SZ)
RUNOUT=$OUT/resume-run-$STAMP.out
RUNERR=$OUT/resume-run-$STAMP.err
sudo -v   # refresh the sudo timestamp in the foreground: a detached sudo cannot ask for a password
: >"$RUNOUT"; : >"$RUNERR"
$R setsid sh -c "timeout 5400 $BIN/rusty_orch lens run $CFG --bbp $BIN/bbp --dir $D --task $TASK \
  --state /srv/lel/orch-state --experiment $TASK --repo $TOY/repo --max-passes 8; echo \"lens run exit=\$?\"" \
  >"$RUNOUT" 2>"$RUNERR" </dev/null &
disown
sleep 5
echo "started detached; output $RUNOUT, errors $RUNERR"
wait_for_end
echo "--- $RUNOUT (the last line is the exit code; 124 = the 90-minute box stopped it)"; cat "$RUNOUT"
echo "--- tail of $RUNERR"; tail -n 30 "$RUNERR" || true
tail -n 1 "$RUNOUT" | grep -q "lens run exit=0" || echo "NOTE: the run did not exit 0; read the lines above before trusting the read-back"

# ---- 3. Read back
echo "=== 3. read back"
$R $BIN/bbp card   --dir $D --task "$TASK"                | tee "$OUT/card-after.json"
$R $BIN/bbp verify --dir $D --task "$TASK"                | tee "$OUT/verify-after.txt"
$R $BIN/bbp why    --dir $D --task "$TASK" --claim "$A"   | tee "$OUT/why-A.json"
$R $BIN/bbp why    --dir $D --task "$TASK" --claim "$Q"   | tee "$OUT/why-Q.json"
[[ ${A2:-} =~ ^[0-9]+$ ]] && $R $BIN/bbp why --dir $D --task "$TASK" --claim "$A2" | tee "$OUT/why-A2.json"
$R $BIN/bbp inbox  --dir $D --task "$TASK" --json yes     | tee "$OUT/inbox.json"
$R $BIN/rusty_orch lens record --bbp $BIN/bbp --dir $D --task "$TASK" --state /srv/lel/orch-state | tee "$OUT/lens-record.txt"
bash "$HERE/1166-grounding.sh" "$D" "$TASK" "$OUT/grounding.md" "$BIN" > /dev/null || echo "WARNING: the grounding bundle failed: do not act on an escalated claim until it is produced"   # D155
echo "grounding bundle: $OUT/grounding.md (Claude writes the audit from it BEFORE you act on any escalated claim)"
echo "--- AVC denials (none expected)"
sudo ausearch -m avc -ts recent 2>/dev/null | tail -20 | tee "$OUT/avc.txt" || true

cat <<EOF

=== Done. Nothing is deleted at any point. Files are in $OUT
Next, by hand:
  0. Do NOT act on an escalated claim yet: send Claude $OUT/grounding.md first; the grounding audit (D155) comes before any tap.
  1. Inbox: A (msg $A) should have escalated 'irreversible'. In the Mill Board inbox tap Reject on A
     (or the CLI: sudo -H -u orch-runner $BIN/bbp human --dir $D --task $TASK --dry-run yes claim $A drop, then without --dry-run).
     If Q escalated 'stuck', choose one candidate. If the lenses never pushed A to the line, that is a finding: do not edit A by hand.
  2. 'bbp why --claim' for one candidate: take its id from $OUT/card-after.json.
  3. Send Claude: $OUT/resume-run-*.out and .err, card-resume-start.json, card-after.json, verify-before.txt, verify-after.txt,
     lens-record.txt, why-A.json, why-Q.json, inbox.json, avc.txt, and the 'llama-swap loaded' lines above.
EOF
