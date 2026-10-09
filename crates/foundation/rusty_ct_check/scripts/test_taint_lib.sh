#!/bin/sh
# Regression test for taint_lib.sh with a fake `valgrind`: an example that exits
# non-zero before printing any diagnostic must FAIL the check, not read as
# "0 errors" (Codex round 3 on #540). Needs only a POSIX sh.
set -eu
HERE=$(cd "$(dirname "$0")" && pwd)
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
mkdir "$TMP/bin"
BIN="$TMP/example"; : >"$BIN"

fake() { # fake EXIT_CODE [OUTPUT-LINE]: install a `valgrind` that prints the line and exits
  printf '#!/bin/sh\n[ -n "%s" ] && echo "%s"\nexit %s\n' "${2:-}" "${2:-}" "$1" >"$TMP/bin/valgrind"
  chmod +x "$TMP/bin/valgrind"
}
# check NAME EXPECT_FAIL EXPECT_COUNT MODE: run run_mode in a subshell and compare.
check() {
  ( PATH="$TMP/bin:$PATH"; fail=0; . "$HERE/taint_lib.sh"; run_mode m >/dev/null
    n=$(count m); echo "$fail $n" ) | {
    read -r got_fail got_n
    [ "$got_fail" = "$2" ] && [ "$got_n" = "$3" ] || { echo "FAIL $1: fail=$got_fail count=$got_n, want fail=$2 count=$3"; exit 1; }
    echo "ok   $1"
  }
}
fake 0;                       check "clean run, no diagnostics"        0 0
fake 0 "==1== Conditional jump or move depends on uninitialised value(s)"
                              check "diagnostic is counted"            0 1
fake 3;                       check "exit 3 before any output fails"   1 0
fake 127;                     check "missing tool (127) fails"         1 0
fake 1 "==1== Conditional jump or move depends on uninitialised value(s)"
                              check "diagnostic + non-zero still fails" 1 1
