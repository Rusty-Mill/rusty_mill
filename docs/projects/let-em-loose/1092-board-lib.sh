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

# service_check ACTIVE USER MAIN_PID [RUNNING] < ss_output: a running mill-board.service runs as mill-board, and
# every listener on the board's port (`ss -Hltnp`, all lines) is that service's main pid on a loopback address.
# A line without a pid, or an address that is not loopback, is unknown ownership and refused. With RUNNING=1
# (after start, before any fixture request) the service must be active and hold at least one listener.
service_check() {
  local active=$1 user=$2 main=$3 running=${4:-0} line addr pids p seen=0
  if [ "$active" = active ] && [ "$user" != "$BOARD_USER" ]; then
    refuse "mill-board.service runs as '${user:-root}', not $BOARD_USER"; return 1
  fi
  while IFS= read -r line; do
    [ -n "$line" ] || continue
    addr=$(awk '{print $4}' <<<"$line")
    loopback_check "$addr" || return 1
    pids=$(grep -o 'pid=[0-9]*' <<<"$line" | cut -d= -f2)
    [ -n "$pids" ] || refuse "a listener on the board's port has no visible owner: $line" || return 1
    for p in $pids; do
      [ "$active" = active ] && [ "$p" = "$main" ] \
        || refuse "the board's port is held by pid $p, not mill-board.service (main pid ${main:-none})" || return 1
    done
    seen=1
  done
  [ "$running" = 0 ] && return 0
  [ "$active" = active ] || refuse "mill-board.service is '$active' after start" || return 1
  [ "$seen" = 1 ] || refuse "mill-board.service holds no listener on the board's port"
}

# The properties the approved unit is compared on, and how `systemctl show` is asked for them.
UNIT_PROPS="LoadState FragmentPath DropInPaths User EnvironmentFiles Environment PassEnvironment UnsetEnvironment
ExecCondition ExecStartPre ExecStart ExecStartPost ExecReload ExecStop ExecStopPost"

# unit_expected UNIT STATE BIN LIBEXEC DIR: the approved effective unit, one PROP=VALUE per line in UNIT_PROPS
# order: the shipped fragment, only this script's drop-in, mill-board, only board.env, exactly the drop-in's
# Environment, no passed or unset variables, ExecStart with no arguments, and no other Exec hook.
unit_expected() {
  local unit=$1 state=$2 bin=$3 libexec=$4 dir=$5
  printf '%s\n' LoadState=loaded "FragmentPath=$unit" "DropInPaths=$unit.d/lel.conf" "User=$BOARD_USER" \
    "EnvironmentFiles=$state/board.env" \
    "Environment=MILL_BOARD_BBP_BIN=$libexec/mill-board-bbp MILL_BOARD_BBP_ROOT=$dir" \
    PassEnvironment= UnsetEnvironment= ExecCondition= ExecStartPre= "ExecStart=$bin/rusty_mill_board" \
    ExecStartPost= ExecReload= ExecStop= ExecStopPost=
}

# exec_argv VALUE: a `systemctl show` Exec value as its argv, or `<invalid>` unless it is exactly one command
# whose argv[0] is its path. Empty stays empty.
exec_argv() {
  local v=$1 path argv
  [ -n "$v" ] || return 0
  if [ "$(grep -o '{ path=' <<<"$v" | wc -l)" != 1 ]; then echo '<invalid: not exactly one command>'; return 0; fi
  path=$(sed -n 's/^{ path=\([^ ]*\) ;.*/\1/p' <<<"$v")
  argv=$(sed -n 's/.* argv\[\]=\(.*\) ; ignore_errors=.*/\1/p' <<<"$v")
  [ -n "$path" ] && [ "${argv%% *}" = "$path" ] || { echo '<invalid: argv does not start with path>'; return 0; }
  echo "$argv"
}

# unit_normalize < `systemctl show` output: UNIT_PROPS as PROP=VALUE lines, missing ones empty, Exec values as
# their argv, environment files without their ignore_errors flags.
unit_normalize() {
  local show k v
  show=$(cat)
  for k in $UNIT_PROPS; do
    v=$(env_value "$k" <<<"$show")
    case $k in
      Exec*) v=$(exec_argv "$v") ;;
      EnvironmentFiles) v=${v// (ignore_errors=no)/}; v=${v// (ignore_errors=yes)/} ;;
    esac
    echo "$k=$v"
  done
}

# unit_effective_check EXPECTED LOADED < `systemctl show` output: what systemd will actually run equals the
# approved unit (unit_expected) on every property in UNIT_PROPS. With LOADED=0 a unit that does not exist yet
# passes. Any difference is refused, naming the property.
unit_effective_check() {
  local expected=$1 loaded=$2 show got diff
  show=$(cat)
  if [ "$(env_value LoadState <<<"$show")" = not-found ]; then
    [ "$loaded" = 0 ] || refuse "mill-board.service is not loaded after install" || return 1
    return 0
  fi
  got=$(unit_normalize <<<"$show")
  diff=$(diff <(echo "$expected") <(echo "$got") | sed -n 's/^> /  got /p; s/^< /  want /p') || true
  [ -z "$diff" ] || refuse "mill-board.service is not the approved unit:"$'\n'"$diff"
}

# lookup_state STATUS STDERR -> exists | absent. `bbp card` exited STATUS; only bbp's own unknown-task error
# counts as absent. Any other failure is a failed lookup, not an absent task.
lookup_state() {
  if [ "$1" = 0 ]; then echo exists; return 0; fi
  grep -qx 'bbp: UnknownTask' <<<"$2" && { echo absent; return 0; }
  refuse "bbp could not look up $FIXTURE_TASK (exit $1): $(tail -n 1 <<<"$2")"
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

# dry_run_ok BODY: inbox_act's dry run answered, BBP would accept the first command, and it would write events.
dry_run_ok() {
  jq -e '(.dry_run.response | has("Accepted")) and (.dry_run.events | length) > 0' <<<"$1" >/dev/null 2>&1 \
    || refuse "the dry run is not an accepted preview with events; no real request sent"
}

FIXTURE_ROLES="seed architect adversary operator seed-s1 seed-s2"   # the toy roster, as 1166-smoke-run.sh assigns it

# fixture_seed: open the fixture task, confirm bbp now has it, assign the roster, seed A2; prints A2's id.
# Each step's status is checked here, because errexit does not apply inside a command substitution.
fixture_seed() {
  local r a2 tstate
  hook_open || refuse "bbp open $FIXTURE_TASK failed; nothing assigned or seeded" || return 1
  tstate=$(hook_task_state) || return 1
  [ "$tstate" = exists ] || refuse "bbp open reported success but $FIXTURE_TASK is '$tstate'" || return 1
  for r in $FIXTURE_ROLES; do
    hook_assign "$r" || refuse "bbp assign $r failed; A2 not seeded" || return 1
  done
  a2=$(hook_seed_a2) || refuse "seeding A2 failed" || return 1
  [[ $a2 =~ ^[0-9]+$ ]] || refuse "seeding returned no A2 id" || return 1
  echo "$a2"
}

# fixture_verify: verification step 4 on the script-owned fixture. The caller defines the hooks:
#   hook_marker_read, hook_marker_write JSON, hook_task_state (prints exists|absent; fails on a failed lookup),
#   hook_open, hook_assign ROLE, hook_seed_a2 (prints A2's id) for fixture_seed,
#   hook_api OP JSON (prints the body), hook_bbp_open (prints bbp's open row count for the task).
# A completed fixture is a no-op: it only re-reads that nothing is open.
fixture_verify() {
  local m exists=0 tstate plan a2 card row act out left bleft
  m=$(hook_marker_read) || refuse "the fixture marker could not be read" || return 1
  tstate=$(hook_task_state) || return 1
  case $tstate in exists) exists=1 ;; absent) exists=0 ;; *) refuse "task lookup said '$tstate'"; return 1 ;; esac
  plan=$(fixture_plan "$m" "$exists") || return 1
  if [ "$plan" = "done" ]; then
    bleft=$(hook_bbp_open) || return 1
    [ "$bleft" = 0 ] || refuse "$FIXTURE_TASK is recorded as verified but bbp shows $bleft open" || return 1
    echo "ok: $FIXTURE_TASK already verified; nothing to do"; return 0
  fi
  if [ "$plan" = create ]; then
    a2=$(fixture_seed) || return 1
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
  out=$(hook_api inbox_act "$(jq -c '. + {dry_run: true}' <<<"$act")") || refuse "the dry run failed" || return 1
  echo "dry run: $out"
  dry_run_ok "$out" || return 1
  out=$(hook_api inbox_act "$act") || refuse "the reject request failed" || return 1
  echo "reject:  $out"
  jq -e '.outcome == "Done"' <<<"$out" >/dev/null 2>&1 || refuse "the reject did not end Done" || return 1
  left=$(hook_api inbox_list '{}' | jq --arg t "$FIXTURE_TASK" '[.[] | select(.task == $t)] | length') || return 1
  bleft=$(hook_bbp_open) || return 1
  [ "$left" = 0 ] && [ "$bleft" = 0 ] || refuse "escalation still open (board $left, bbp $bleft)" || return 1
  hook_marker_write "$(jq -c '.state = "verified"' <<<"$m")" || return 1
  echo "ok: inbox_list and bbp inbox are empty for $FIXTURE_TASK"
}
