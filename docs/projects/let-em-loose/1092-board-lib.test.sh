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
L1='LISTEN 0 128 127.0.0.1:8787 0.0.0.0:* users:(("rusty_mill_boar",pid=42,fd=9))'
sc() { service_check "$1" "$2" "$3" <<<"$4"; }
ok sc inactive "" 0 ""
ok sc active mill-board 42 "$L1"
ok sc active mill-board 42 ""
no sc active root 42 "$L1"
no sc active mill-board 77 "$L1"                                   # someone else holds the port
no sc inactive "" 0 "$L1"
no sc active mill-board 42 "$L1"$'\n''LISTEN 0 128 127.0.0.1:8787 0.0.0.0:* users:(("nc",pid=77,fd=3))'   # a second listener
no sc active mill-board 42 'LISTEN 0 128 127.0.0.1:8787 0.0.0.0:*'                                   # owner not visible
no sc active mill-board 42 'LISTEN 0 128 0.0.0.0:8787 0.0.0.0:* users:(("rusty_mill_boar",pid=42,fd=9))'

# ---- effective unit (systemctl show): the approved configuration, compared property by property
B=/opt/lel/current/bin
SU=/etc/systemd/system/mill-board.service
LX=/usr/local/libexec
DD=/srv/lel/bbp
EXP=$(unit_expected "$SU" "$S" "$B" "$LX" "$DD")
X="{ path=$B/rusty_mill_board ; argv[]=$B/rusty_mill_board ; ignore_errors=no ; start_time=[n/a] ; stop_time=[n/a] ; pid=0 ; code=(null) ; status=0/0 }"
show() {   # show [KEY=VALUE overrides...]: `systemctl show` of the unit as this script installs it
  local base="LoadState=loaded
FragmentPath=$SU
DropInPaths=$SU.d/lel.conf
User=mill-board
EnvironmentFiles=$S/board.env (ignore_errors=no)
Environment=MILL_BOARD_BBP_BIN=$LX/mill-board-bbp MILL_BOARD_BBP_ROOT=$DD
ExecStart=$X" kv
  for kv in "$@"; do base=$(sed "/^${kv%%=*}=/d" <<<"$base"); base+=$'\n'"$kv"; done
  echo "$base"
}
uec() { unit_effective_check "$EXP" "$1" <<<"$2"; }
ok uec 0 'LoadState=not-found'
no uec 1 'LoadState=not-found'
ok uec 1 "$(show)"
ok uec 0 "$(show)"
ok uec 0 "$(show)"$'\n'"ExecStartPre="$'\n'"PassEnvironment="     # empty properties printed explicitly
no uec 0 "$(show LoadState=masked)"
no uec 0 "$(show "DropInPaths=$SU.d/lel.conf /etc/systemd/system/service.d/10-all.conf")"
no uec 0 "$(show FragmentPath=/usr/lib/systemd/system/mill-board.service)"
no uec 0 "$(show User=root)"
# D132: the unit may not change the service's groups; the account check alone would miss these
no uec 0 "$(show Group=orch-runner)"
no uec 0 "$(show SupplementaryGroups=orch-runner)"
no uec 0 "$(show "SupplementaryGroups=wheel orch-runner")"
no uec 0 "$(show Group=mill-board)"                                 # any unit-set group differs from the approved unit
ok uec 0 "$(show)"$'\n''Group='$'\n''SupplementaryGroups='       # empty, as systemctl prints an unset property
no uec 0 "$(show "EnvironmentFiles=$S/board.env (ignore_errors=no) /etc/other.env (ignore_errors=yes)")"
no uec 0 "$(show 'ExecStart={ path=/usr/bin/other ; argv[]=/usr/bin/other ; ignore_errors=no }')"
# a noncanonical unit with the right executable: arguments, a second command, hooks, environment
no uec 0 "$(show "ExecStart={ path=$B/rusty_mill_board ; argv[]=$B/rusty_mill_board --http 0.0.0.0:8787 ; ignore_errors=no }")"
no uec 0 "$(show "ExecStart={ path=$B/rusty_mill_board ; argv[]=/bin/sh -c x ; ignore_errors=no }")"
no uec 0 "$(show "ExecStart=$X ; { path=/bin/sh ; argv[]=/bin/sh -c x ; ignore_errors=no }")"
no uec 0 "$(show 'ExecStartPre={ path=/bin/sh ; argv[]=/bin/sh -c x ; ignore_errors=no }')"
no uec 0 "$(show 'ExecStartPost={ path=/bin/sh ; argv[]=/bin/sh -c x ; ignore_errors=no }')"
no uec 0 "$(show 'ExecCondition={ path=/bin/true ; argv[]=/bin/true ; ignore_errors=no }')"
no uec 0 "$(show 'ExecStopPost={ path=/bin/sh ; argv[]=/bin/sh -c x ; ignore_errors=no }')"
no uec 0 "$(show "Environment=MILL_BOARD_BBP_BIN=/tmp/fake MILL_BOARD_BBP_ROOT=$DD")"
no uec 0 "$(show "Environment=MILL_BOARD_BBP_BIN=$LX/mill-board-bbp MILL_BOARD_BBP_ROOT=$DD MILL_BOARD_HTTP=0.0.0.0:8787")"
no uec 0 "$(show PassEnvironment=MILL_BOARD_HTTP)"
no uec 0 "$(show UnsetEnvironment=MILL_BOARD_DB)"

# ---- after start, before any fixture request: the service is up and holds the port
scr() { service_check "$1" "$2" "$3" 1 <<<"$4"; }
ok scr active mill-board 42 "$L1"
no scr active mill-board 42 ""                                     # running but not listening
no scr active mill-board 77 "$L1"                                  # the port belongs to another process
no scr failed mill-board 42 ""
no scr activating mill-board 42 "$L1"

# ---- lookup_state: absent only on bbp's own unknown-task error
eq "$(lookup_state 0 '')" exists "lookup: exit 0 -> exists"
eq "$(lookup_state 1 'bbp: UnknownTask')" absent "lookup: UnknownTask -> absent"
no lookup_state 1 'bbp: master: Permission denied'
no lookup_state 1 'sudo: a password is required'
no lookup_state 1 ''

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
DRY_OK='{"dry_run":{"step":{},"response":{"Accepted":{"Ok":null}},"events":[{"ClaimDecided":{}}],"then":[]}}'
reset() { rm -f "$T/lookup_fail" "$T/open_fail" "$T/open_partial" "$T/assign_fail"; echo 11 >"$T/a2"; echo "$DRY_OK" >"$T/dry"; : >"$T/marker"; printf '%s\n' "$@" >"$T/tasks"; echo 0 >"$T/open"; : >"$T/calls"; echo 0 >"$T/card"; }
hook_marker_read()  { cat "$T/marker"; }
hook_marker_write() { echo "$1" >"$T/marker"; }
hook_task_state()   { if [ -e "$T/lookup_fail" ]; then echo "lookup failed" >&2; return 1; fi
                      if grep -qx "$FIXTURE_TASK" "$T/tasks"; then echo exists; else echo absent; fi; }
hook_open()         { echo "open" >>"$T/calls"; [ ! -e "$T/open_partial" ] || { echo "$FIXTURE_TASK" >>"$T/tasks"; return 1; }
                      [ ! -e "$T/open_fail" ] || return 1; echo "$FIXTURE_TASK" >>"$T/tasks"; }
hook_assign()       { echo "assign $1" >>"$T/calls"; [ "$1" != "$(cat "$T/assign_fail" 2>/dev/null)" ]; }
hook_seed_a2()      { echo "seed" >>"$T/calls"; echo 1 >"$T/open"; cat "$T/a2"; }
hook_bbp_open()     { cat "$T/open"; }
hook_api() {
  case $1 in
    inbox_list) if [ "$(cat "$T/open")" = 1 ]; then echo "[$(row lel-1092-fixture "$(cat "$T/card")" 11 false),$(row board-check 1 5 false)]"; else echo "[$(row board-check 1 5 false)]"; fi ;;
    card_add) echo 3 >"$T/card"; echo "$1" >>"$T/calls"; echo '{"card":{"number":3}}' ;;
    inbox_act) echo "$1 $2" >>"$T/calls"
      if jq -e .dry_run <<<"$2" >/dev/null; then cat "$T/dry"; return; fi
      jq -e '.id | startswith("lel-1092-fixture/")' <<<"$2" >/dev/null && echo 0 >"$T/open"
      echo '{"outcome":"Done"}' ;;
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

# ---- dry_run_ok
ok dry_run_ok "$DRY_OK"
no dry_run_ok '{"dry_run":{"response":{"Rejected":{"code":"StaleRev","detail":"x"}},"events":[]}}'
no dry_run_ok '{"dry_run":{"response":{"Accepted":{}},"events":[]}}'
no dry_run_ok ''
no dry_run_ok '{"outcome":"Done"}'

reset board-check                           # a refused dry run: the real reject is never sent
echo '{"dry_run":{"response":{"Rejected":{"code":"StaleRev","detail":"x"}},"events":[]}}' >"$T/dry"
no fixture_verify
eq "$(grep -c '^inbox_act' "$T/calls")" 1 "refused dry run: only the dry run was sent"
eq "$(jq -r .state "$T/marker")" linked "refused dry run: marker stays linked"

reset board-check                           # the fixture row changed under us: no real reject
echo 12 >"$T/a2"
no fixture_verify
eq "$(grep -c '^inbox_act' "$T/calls" || true)" 0 "identity mismatch: no inbox_act at all"

# ---- fail closed on creation and lookup
reset board-check; touch "$T/open_fail"     # bbp open fails
no fixture_verify
eq "$(cat "$T/calls")" "open" "failed create: no assign, seed, card or inbox call"
eq "$(cat "$T/marker")" "" "failed create: nothing recorded"

reset board-check; touch "$T/open_partial"  # bbp open fails after writing the task
no fixture_verify
eq "$(cat "$T/calls")" "open" "failed create that left a task: no assign, seed, card or inbox call"
eq "$(cat "$T/marker")" "" "failed create that left a task: nothing recorded"

reset board-check; echo architect >"$T/assign_fail"   # an assignment fails
no fixture_verify
eq "$(grep -c '^seed\|^card_add\|^inbox_act' "$T/calls" || true)" 0 "failed assign: no seed, card or inbox_act"
eq "$(tail -n 1 "$T/calls")" "assign architect" "failed assign: stops at the failing role"

reset board-check; echo "not-a-number" >"$T/a2"       # seeding prints no id
no fixture_verify
eq "$(grep -c '^card_add\|^inbox_act' "$T/calls" || true)" 0 "no A2 id: no card or inbox_act"

reset board-check; touch "$T/lookup_fail"             # a failed lookup is not an absent task
no fixture_verify
eq "$(cat "$T/calls")" "" "failed lookup: nothing opened or sent"

reset board-check                                     # open succeeds but the task is still not there
hook_open() { echo "open" >>"$T/calls"; }
no fixture_verify
eq "$(cat "$T/calls")" "open" "open without a task: no assign, seed or inbox call"

echo
if [ "$fails" != 0 ]; then echo "$fails failed"; exit 1; fi
echo "all passed"
