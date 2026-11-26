# Sourced by 1318-toy-run.sh and 1318-toy-verdict.test.sh. Bash; needs jq.
# toy_verdict CLAIM ROW1 ROW2 SESSIONS: the D197 item 6 session check. ROW1 and ROW2 are the two
# `rusty_orch builder run` stdout rows; SESSIONS is `rusty_orch builder sessions --claim CLAIM` output,
# one JSON row per persisted session. Passes (returns 0) only when both stdout rows are terminal by D188
# (stored or failed) with no save error, their n differ, and the store holds exactly those two sessions
# for CLAIM in the same states with the same candidates. Anything unreadable, missing or different fails
# closed (returns 1) with one FAIL line per reason on stdout.
toy_verdict() {
  local claim=$1 sessions=$4 fail=0 i f st n ns
  local -a want=()
  [[ $claim =~ ^[0-9]+$ ]] || { echo "FAIL: claim id '$claim' is not a number"; return 1; }
  for i in 1 2; do
    f=${*:$((i + 1)):1}
    if ! st=$(jq -er --argjson c "$claim" '
        if type != "object" then error("not an object")
        elif .claim != $c then error("claim \(.claim), not \($c)")
        elif (.n | type) != "number" then error("no n")
        elif (.state | IN("stored", "failed") | not) then error("state \(.state)")
        elif ([.notes[]? | strings | select(startswith("save:"))] | length) > 0 then error("final save failed")
        else "\(.n) \(.state) \(.candidate)" end' "$f" 2>&1); then
      echo "FAIL: builder $i stdout row: ${st:-unreadable} ($f)"; fail=1; continue
    fi
    want+=("$st")
  done
  if [ "${#want[@]}" = 2 ] && [ "${want[0]%% *}" = "${want[1]%% *}" ]; then
    echo "FAIL: both builders report session n ${want[0]%% *}"; fail=1
  fi
  if ! ns=$(jq -esr --argjson c "$claim" '
      if all(.[]; type == "object") | not then error("a row is not an object") else . end
      | map(select(.claim == $c)) | map("\(.n) \(.state) \(.candidate)") | sort | .[]' "$sessions" 2>&1); then
    echo "FAIL: persisted sessions unreadable: ${ns:-empty} ($sessions)"; return 1
  fi
  n=$(grep -c . <<<"$ns" || true)
  [ "$n" = 2 ] || { echo "FAIL: the store holds $n sessions for claim $claim, not 2: ${ns//$'\n'/; }"; fail=1; }
  if [ "${#want[@]}" = 2 ]; then
    local expect; expect=$(printf '%s\n' "${want[@]}" | LC_ALL=C sort)
    [ "$expect" = "$ns" ] || { echo "FAIL: persisted sessions (${ns//$'\n'/; }) differ from stdout (${expect//$'\n'/; })"; fail=1; }
  fi
  return "$fail"
}
