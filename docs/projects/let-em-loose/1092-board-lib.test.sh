#!/bin/bash
# Tests for 1092-board-lib.sh. Isolated: temp files and in-memory fakes, no sudo, no bbp, no board, no network.
#   bash docs/projects/let-em-loose/1092-board-lib.test.sh
set -euo pipefail
unset CDPATH
# shellcheck source=1092-board-lib.sh
. "$(dirname "$0")/1092-board-lib.sh"

T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
fails=0
ok()  { if "$@" 2>/dev/null >/dev/null; then echo "ok   ${*//$'\n'/ }"; else echo "FAIL expected pass: ${*//$'\n'/ }"; fails=$((fails + 1)); fi; }
no()  { if "$@" 2>/dev/null >/dev/null; then echo "FAIL expected refusal: ${*//$'\n'/ }"; fails=$((fails + 1)); else echo "ok   refused: ${*//$'\n'/ }"; fi; }
eq()  { if [ "$1" = "$2" ]; then echo "ok   $3"; else echo "FAIL $3: got '$1', want '$2'"; fails=$((fails + 1)); fi; }

# ---- host and listener
ok host_check BaileyAi baileyai
ok host_check baileyai.lan baileyai
no host_check ms-01 baileyai
ok loopback_check 127.0.0.1:8787
ok loopback_check '[::1]:8787'
no loopback_check 0.0.0.0:8787
no loopback_check 192.168.1.20:8787

# ---- board.env: checked, never rewritten
S=/var/lib/mill-board
env_ok() { printf 'MILL_BOARD_SETTINGS_DIR=%s/settings\nMILL_BOARD_DB=%s/board.db\n%s' "$S" "$S" "${1-}"; }
envc() { env_check "$S" <<<"$1"; }
ok envc "$(env_ok)"
ok envc "$(env_ok 'MILL_BOARD_HTTP=127.0.0.1:9000')"
no envc "$(env_ok 'MILL_BOARD_HTTP=0.0.0.0:8787')"
no envc "$(env_ok 'MILL_BOARD_TICK_URL=http://127.0.0.1:8788')"
no envc "$(env_ok 'MILL_BOARD_DB=/srv/live/board.db')"
no envc "MILL_BOARD_SETTINGS_DIR=$S/settings"

# ---- unit: fresh, ours, or refused
U=$T/unit/mill-board.service
mkdir -p "$T/unit"
ok unit_check "$U" "$S"
mkdir -p "$U.d"; ok unit_check "$U" "$S"
echo '[Service]' >"$U"
no unit_check "$U" "$S"                       # someone else's unit, no drop-in
printf '[Service]\nUser=other\nEnvironmentFile=/etc/other.env\n' >"$U.d/lel.conf"
no unit_check "$U" "$S"                       # a lel.conf that is not ours
printf '[Service]\nUser=mill-board\nEnvironmentFile=\nEnvironmentFile=%s/board.env\n' "$S" >"$U.d/lel.conf"
ok unit_check "$U" "$S"
echo '[Service]' >"$U.d/zz-local.conf"
no unit_check "$U" "$S"                       # another drop-in could override ours
rm "$U.d/zz-local.conf"; rm "$U"
no unit_check "$U" "$S"                       # drop-ins without a unit

# ---- service and port owner
ok service_check inactive "" 0 ""
ok service_check active mill-board 42 42
ok service_check active mill-board 42 ""
no service_check active root 42 42
no service_check active mill-board 42 77
no service_check inactive "" 0 77

# ---- fixture_plan
eq "$(fixture_plan "" 0)" create "plan: nothing recorded, no task -> create"
no fixture_plan "" 1                          # pre-existing task the script did not record
no fixture_plan '{"task":"board-check","state":"verified"}' 1
no fixture_plan '{"task":"lel-1092-fixture","state":"seeded"}' 0
no fixture_plan 'not json' 1
eq "$(fixture_plan '{"task":"lel-1092-fixture","state":"linked"}' 1)" resume "plan: linked -> resume"
eq "$(fixture_plan '{"task":"lel-1092-fixture","state":"verified"}' 1)" "done" "plan: verified -> done"

# ---- fixture_row: exact identity
M='{"task":"lel-1092-fixture","a2":11,"card":3,"state":"linked"}'
row() {   # row TASK CARD CLAIM REVERSIBLE
  jq -nc --arg t "$1" --argjson c "$2" --argjson a "$3" --argjson r "$4" \
    '{id: "\($t)/\($a)/9", task: $t, rev: 20, seq: 9, reason: "Irreversible", reversible: $r, label: "x",
      claim: {id: $a, text: "Proposed action, not run: run `sweep --yes` on the toy data directory with DAYS 7."},
      card: {number: $c}}'
}
ok fixture_row "[$(row lel-1092-fixture 3 11 false),$(row other 9 5 false)]" "$M"
no fixture_row "[$(row lel-1092-fixture 3 11 false),$(row lel-1092-fixture 3 12 false)]" "$M"
no fixture_row "[$(row lel-1092-fixture 4 11 false)]" "$M"
no fixture_row "[$(row lel-1092-fixture 3 12 false)]" "$M"
no fixture_row "[$(row lel-1092-fixture 3 11 true)]" "$M"
no fixture_row "[]" "$M"

# ---- fixture_verify against an in-memory BBP and board
# State: MARKER (file), TASKS (bbp tasks), OPEN (open escalations of the fixture task), CALLS (api writes).
reset() { : >"$T/marker"; printf '%s\n' "$@" >"$T/tasks"; echo 0 >"$T/open"; : >"$T/calls"; echo 0 >"$T/card"; }
hook_marker_read()  { cat "$T/marker"; }
hook_marker_write() { echo "$1" >"$T/marker"; }
hook_task_exists()  { grep -qx "$FIXTURE_TASK" "$T/tasks"; }
hook_seed()         { echo "$FIXTURE_TASK" >>"$T/tasks"; echo 1 >"$T/open"; echo 11; }
hook_bbp_open()     { cat "$T/open"; }
hook_api() {
  case $1 in
    inbox_list) if [ "$(cat "$T/open")" = 1 ]; then echo "[$(row lel-1092-fixture "$(cat "$T/card")" 11 false),$(row board-check 1 5 false)]"; else echo "[$(row board-check 1 5 false)]"; fi ;;
    card_add) echo 3 >"$T/card"; echo "$1" >>"$T/calls"; echo '{"card":{"number":3}}' ;;
    inbox_act) echo "$1 $2" >>"$T/calls"
      jq -e .dry_run <<<"$2" >/dev/null || { jq -e '.id | startswith("lel-1092-fixture/")' <<<"$2" >/dev/null && echo 0 >"$T/open"; }
      echo '{}' ;;
    *) echo "$1" >>"$T/calls"; echo '{}' ;;
  esac
}

reset board-check
ok fixture_verify
eq "$(jq -r .state "$T/marker")" verified "fresh run: marker verified"
eq "$(grep -c '^inbox_act' "$T/calls")" 2 "fresh run: one dry run and one reject"
eq "$(grep -c 'board-check/' "$T/calls" || true)" 0 "fresh run: the unrelated task's row is never acted on"

: >"$T/calls"
out=$(fixture_verify)
eq "$(cat "$T/calls")" "" "completed rerun: no board writes"
eq "$(grep -c "$FIXTURE_TASK" "$T/tasks")" 1 "completed rerun: not seeded again"
eq "$out" "ok: $FIXTURE_TASK already verified; nothing to do" "completed rerun: reports a no-op"

reset board-check "$FIXTURE_TASK"           # the fixture's name already taken by a task this script did not record
no fixture_verify
eq "$(cat "$T/calls")" "" "pre-existing task: no board writes"
eq "$(cat "$T/marker")" "" "pre-existing task: nothing recorded"

reset board-check                           # resume after a crash between card_add and the link
hook_marker_write '{"task":"lel-1092-fixture","a2":11,"card":3,"state":"carded"}'
echo "$FIXTURE_TASK" >>"$T/tasks"; echo 1 >"$T/open"; echo 3 >"$T/card"
ok fixture_verify
eq "$(grep -c '^card_add' "$T/calls" || true)" 0 "resume: no second card"
eq "$(jq -r .state "$T/marker")" verified "resume: marker verified"

reset board-check                           # the fixture row changed under us: no real reject
hook_seed() { echo "$FIXTURE_TASK" >>"$T/tasks"; echo 1 >"$T/open"; echo 12; }
no fixture_verify
eq "$(grep -c '^inbox_act' "$T/calls" || true)" 0 "identity mismatch: no inbox_act at all"

echo
if [ "$fails" != 0 ]; then echo "$fails failed"; exit 1; fi
echo "all passed"
