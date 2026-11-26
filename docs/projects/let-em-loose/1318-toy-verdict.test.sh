#!/bin/bash
# Offline fixtures for toy_verdict (1318-toy-verdict.sh): no binaries, provider, host or network.
#   bash docs/projects/let-em-loose/1318-toy-verdict.test.sh
set -uo pipefail
. "$(dirname "$0")/1318-toy-verdict.sh"
W=$(mktemp -d); trap 'rm -rf "$W"' EXIT
row() { printf '{"claim":%s,"n":%s,"started":1,"state":"%s","ended":%s,"candidate":%s,"tokens_in":0,"tokens_out":0%s}\n' "$@"; }
r1=$(row 7 1 stored 9 42 ',"notes":[]'); r2=$(row 7 2 failed 9 null ',"notes":["no submit marker or an empty diff: nothing posted"]')
p1=$(row 7 1 stored 9 42 ''); p2=$(row 7 2 failed 9 null '')
bad=0
case_() { # NAME WANT(0 pass, 1 fail) ROW1 ROW2 SESSIONS
  printf '%s\n' "$3" > "$W/1"; printf '%s\n' "$4" > "$W/2"
  if [ "$5" = MISSING ]; then rm -f "$W/s"; else printf '%s' "$5" > "$W/s"; fi
  out=$(toy_verdict 7 "$W/1" "$W/2" "$W/s"); got=$?
  if [ "$got" = "$2" ]; then echo "ok   $1"; else echo "FAIL $1: returned $got, want $2"; bad=1; fi
  [ -z "$out" ] || sed 's/^/       /' <<<"$out"
}
case_ "durable terminal rows match stdout: pass"        0 "$r1" "$r2" "$p1"$'\n'"$p2"$'\n'
case_ "a persisted failed session is a pass (D197)"     0 "$(row 7 1 failed 9 null ',"notes":[]')" "$r2" "$(row 7 1 failed 9 null '')"$'\n'"$p2"$'\n'
case_ "sessions 1 and 11 compare in one order"          0 "$r1" "$(row 7 11 failed 9 null ',"notes":[]')" "$p1"$'\n'"$(row 7 11 failed 9 null '')"$'\n'
case_ "other claims' sessions are ignored"              0 "$r1" "$r2" "$p1"$'\n'"$p2"$'\n'"$(row 8 1 running null null '')"$'\n'
case_ "stdout terminal, persisted running: fail"        1 "$r1" "$r2" "$p1"$'\n'"$(row 7 2 running null null '')"$'\n'
case_ "stdout terminal, persisted missing: fail"        1 "$r1" "$r2" "$p1"$'\n'
case_ "no sessions output at all: fail"                 1 "$r1" "$r2" MISSING
case_ "empty sessions output: fail"                     1 "$r1" "$r2" ""
case_ "unparseable sessions output: fail"               1 "$r1" "$r2" "not json"
case_ "final save error in notes: fail"                 1 "$r1" "$(row 7 2 failed 9 null ',"notes":["save: disk full"]')" "$p1"$'\n'"$p2"$'\n'
case_ "stdout aborted: fail"                            1 "$r1" "$(row 7 2 aborted 9 null ',"notes":[]')" "$p1"$'\n'"$(row 7 2 aborted 9 null '')"$'\n'
case_ "persisted state differs from stdout: fail"       1 "$r1" "$r2" "$p1"$'\n'"$(row 7 2 stored 9 43 '')"$'\n'
case_ "persisted candidate differs: fail"               1 "$r1" "$r2" "$(row 7 1 stored 9 41 '')"$'\n'"$p2"$'\n'
case_ "both builders report one n: fail"                1 "$r1" "$r1" "$p1"$'\n'"$p1"$'\n'
case_ "a third session for the claim: fail"             1 "$r1" "$r2" "$p1"$'\n'"$p2"$'\n'"$(row 7 3 failed 9 null '')"$'\n'
case_ "stdout row empty (builder crashed): fail"        1 "" "$r2" "$p1"$'\n'"$p2"$'\n'
case_ "stdout row for another claim: fail"              1 "$(row 8 1 stored 9 42 ',"notes":[]')" "$r2" "$p1"$'\n'"$p2"$'\n'
exit "$bad"
