# shellcheck shell=bash
# Checks and the fixture flow for 1092-deploy-board.sh. Sourced; defines functions only, no side effects.
# Every check prints why it refuses on stderr and returns 1. Tested by 1092-board-lib.test.sh with fakes.

FIXTURE_TASK=lel-1092-fixture   # fixed: the script only ever acts on the task it created and recorded
# shellcheck disable=SC2016  # literal backticks: A2's text
FIXTURE_TEXT='Proposed action, not run: run `sweep --yes`'   # A2's text, from 1166-seed-inbox.sh
BOARD_USER=mill-board

refuse() { echo "refused: $*" >&2; return 1; }

# host_check NAME WANT: this is the approved host (short name, any case).
host_check() {
  local name=${1,,} want=${2,,}
  [ "${name%%.*}" = "$want" ] || refuse "host is '$1', not '$2' (set LEL_HOST if BaileyAi's hostname differs)"
}

# loopback_check ADDR: the board listens on loopback only.
loopback_check() {
  [[ $1 =~ ^(127\.[0-9]+\.[0-9]+\.[0-9]+|\[::1\]|localhost):[0-9]+$ ]] \
    || refuse "MILL_BOARD_HTTP '$1' is not a loopback address"
}

# env_value KEY < board.env: the last value of KEY.
env_value() { sed -n "s/^$1=//p" | tail -n 1; }

# env_check STATE < board.env: an existing board.env is this inbox board's: its own SQLite file and settings
# under STATE, no tick, a loopback listener. It is checked, never rewritten.
env_check() {
  local state=$1 env db settings http
  env=$(cat)
  db=$(env_value MILL_BOARD_DB <<<"$env")
  settings=$(env_value MILL_BOARD_SETTINGS_DIR <<<"$env")
  http=$(env_value MILL_BOARD_HTTP <<<"$env")
  [ "$db" = "$state/board.db" ] || refuse "board.env has MILL_BOARD_DB '$db', not $state/board.db" || return 1
  [ "$settings" = "$state/settings" ] || refuse "board.env has MILL_BOARD_SETTINGS_DIR '$settings', not $state/settings" || return 1
  if grep -qE '^(MILL_BOARD_TICK_URL|RUSTY_TICK_TOKEN)=' <<<"$env"; then refuse "board.env still points at a tick"; return 1; fi
  loopback_check "${http:-127.0.0.1:8787}"
}

# unit_check UNIT STATE: no unit (fresh), or the unit carries this script's drop-in and no other.
unit_check() {
  local unit=$1 state=$2 d=$1.d extra="" f
  if [ ! -e "$unit" ]; then
    [ ! -e "$d" ] || [ -z "$(ls -A "$d")" ] || refuse "$d exists without $unit" || return 1
    return 0
  fi
  [ -f "$d/lel.conf" ] || refuse "$unit exists without this script's drop-in $d/lel.conf: not ours" || return 1
  grep -qx "User=$BOARD_USER" "$d/lel.conf" && grep -qx "EnvironmentFile=$state/board.env" "$d/lel.conf" \
    || refuse "$d/lel.conf is not this script's (User=$BOARD_USER, EnvironmentFile=$state/board.env)" || return 1
  for f in "$d"/* "$d"/.[!.]*; do
    [ -e "$f" ] && [ "$f" != "$d/lel.conf" ] && extra+="${f##*/} "
  done
  [ -z "$extra" ] || refuse "$d has other drop-ins: $extra"
}

# service_check ACTIVE USER MAIN_PID LISTENER_PID: a running mill-board.service runs as mill-board, and
# whatever holds the board's port is that service (or nothing).
service_check() {
  local active=$1 user=$2 main=$3 listener=$4
  if [ "$active" = active ] && [ "$user" != "$BOARD_USER" ]; then
    refuse "mill-board.service runs as '${user:-root}', not $BOARD_USER"; return 1
  fi
  [ -z "$listener" ] && return 0
  [ "$active" = active ] && [ "$listener" = "$main" ] || refuse "the board's port is held by pid $listener, not mill-board.service"
}

# fixture_plan MARKER EXISTS -> create | resume | done. MARKER is the recorded JSON (empty if none);
# EXISTS is 1 when bbp knows the task. A task this script did not record is never touched.
fixture_plan() {
  local m=$1 exists=$2 task state
  if [ -z "$m" ]; then
    [ "$exists" = 0 ] || refuse "task $FIXTURE_TASK exists but this script did not create it" || return 1
    echo create; return 0
  fi
  task=$(jq -r '.task // empty' <<<"$m" 2>/dev/null) || refuse "the fixture marker is not JSON" || return 1
  state=$(jq -r '.state // empty' <<<"$m")
  [ "$task" = "$FIXTURE_TASK" ] || refuse "the fixture marker names task '$task', not $FIXTURE_TASK" || return 1
  [ "$exists" = 1 ] || refuse "the marker records $FIXTURE_TASK but bbp has no such task" || return 1
  case $state in
    verified) echo "done" ;;
    seeded|carded|linked) echo resume ;;
    *) refuse "the fixture marker has state '$state'" ;;
  esac
}

# fixture_row ROWS MARKER: the task's only inbox row, and it is exactly the recorded fixture: the recorded
# card and A2 claim, irreversible, with A2's text. Prints the row.
fixture_row() {
  local rows=$1 m=$2 mine n
  mine=$(jq -c --arg t "$FIXTURE_TASK" '[.[] | select(.task == $t)]' <<<"$rows")
  n=$(jq length <<<"$mine")
  [ "$n" = 1 ] || refuse "inbox_list shows $n rows for $FIXTURE_TASK, expected exactly 1" || return 1
  jq -e --argjson m "$m" --arg text "$FIXTURE_TEXT" '.[0]
    | .card.number == $m.card and .claim.id == $m.a2 and .reason == "Irreversible"
      and .reversible == false and (.claim.text | startswith($text))' <<<"$mine" >/dev/null \
    || refuse "the $FIXTURE_TASK row is not the recorded fixture (card #$(jq -r .card <<<"$m"), claim $(jq -r .a2 <<<"$m"))" \
    || return 1
  jq -c '.[0]' <<<"$mine"
}

# fixture_verify: verification step 4 on the script-owned fixture. The caller defines the hooks:
#   hook_marker_read, hook_marker_write JSON, hook_task_exists, hook_seed (prints A2's id),
#   hook_api OP JSON (prints the body), hook_bbp_open (prints bbp's open row count for the task).
# A completed fixture is a no-op: it only re-reads that nothing is open.
fixture_verify() {
  local m exists=0 plan a2 card row act out left bleft
  m=$(hook_marker_read)
  if hook_task_exists; then exists=1; fi
  plan=$(fixture_plan "$m" "$exists") || return 1
  if [ "$plan" = "done" ]; then
    bleft=$(hook_bbp_open) || return 1
    [ "$bleft" = 0 ] || refuse "$FIXTURE_TASK is recorded as verified but bbp shows $bleft open" || return 1
    echo "ok: $FIXTURE_TASK already verified; nothing to do"; return 0
  fi
  if [ "$plan" = create ]; then
    a2=$(hook_seed) || return 1
    [[ $a2 =~ ^[0-9]+$ ]] || refuse "seeding returned no A2 id" || return 1
    m=$(jq -nc --arg t "$FIXTURE_TASK" --argjson a "$a2" '{task: $t, a2: $a, state: "seeded"}')
    hook_marker_write "$m" || return 1
  fi
  if [ "$(jq -r .state <<<"$m")" = seeded ]; then
    card=$(hook_api card_add "$(jq -nc --arg t "$FIXTURE_TASK" \
      '{title: "\($t): #1092 inbox verification", body: "Linked to BBP task \($t) (A2 fixture, D146). Verification step 4 of 1092-deploy-layout.md."}')" \
      | jq -r '.card.number // .number // empty') || return 1
    [[ $card =~ ^[0-9]+$ ]] || refuse "card_add returned no card number" || return 1
    m=$(jq -c --argjson c "$card" '.card = $c | .state = "carded"' <<<"$m")
    hook_marker_write "$m" || return 1
  fi
  if [ "$(jq -r .state <<<"$m")" = carded ]; then
    hook_api card_execution_link "$(jq -nc --argjson m "$m" \
      '{id: ($m.card | tostring), execution: $m.task, goal: $m.task, bbp_task: $m.task, attempt: "1", assignment: $m.task, generation: 1}')" \
      >/dev/null || return 1
    m=$(jq -c '.state = "linked"' <<<"$m")
    hook_marker_write "$m" || return 1
    echo "card #$(jq -r .card <<<"$m") linked to $FIXTURE_TASK"
  fi
  row=$(fixture_row "$(hook_api inbox_list '{}')" "$m") || return 1
  echo "inbox_list: $(jq -c '{id, rev, reason, label, card: .card.number}' <<<"$row")"
  act=$(jq -c '{id, rev, action: "reject"}' <<<"$row")
  out=$(hook_api inbox_act "$(jq -c '. + {dry_run: true}' <<<"$act")") || return 1
  echo "dry run: $out"
  out=$(hook_api inbox_act "$act") || return 1
  echo "reject:  $out"
  left=$(hook_api inbox_list '{}' | jq --arg t "$FIXTURE_TASK" '[.[] | select(.task == $t)] | length') || return 1
  bleft=$(hook_bbp_open) || return 1
  [ "$left" = 0 ] && [ "$bleft" = 0 ] || refuse "escalation still open (board $left, bbp $bleft)" || return 1
  hook_marker_write "$(jq -c '.state = "verified"' <<<"$m")" || return 1
  echo "ok: inbox_list and bbp inbox are empty for $FIXTURE_TASK"
}
