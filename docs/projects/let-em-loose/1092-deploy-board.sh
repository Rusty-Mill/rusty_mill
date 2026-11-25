#!/bin/bash
# #1092 Mill Board on BaileyAi with the decision inbox (D132): the `mill-board` user, the bbp guard,
# the sudoers rule and the systemd drop-in from 1092-deploy-layout.md ("Users", "systemd", "Decision"),
# then verification steps 1, 4 and 5 on a toy escalation. No model is called ($0).
# A separate inbox board on BaileyAi with its own SQLite file (Nano, 2026-11-25); the live board is untouched.
# Run from a checkout as baileyrd, after 1092-deploy.sh at the same commit:
#   bash docs/projects/let-em-loose/1092-deploy-board.sh
# Every check runs before anything changes, on every run: this host, and an existing unit, board.env, data and
# listener that are this inbox board's (loopback only). A foreign or non-loopback installation stops the script.
# board.env is created once and then only checked. Files are installed and the service restarted only when they
# changed. Step 4 acts only on the fixture task this script created and recorded; a completed rerun is a no-op.
# Never prints board.env. Never touches /srv/lel/experiment, lel-codex or a task it did not create.
set -euo pipefail
unset CDPATH   # a set CDPATH makes `cd` print the directory

REPO=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
HERE=$REPO/docs/projects/let-em-loose
SHA=$(git -C "$REPO" rev-parse --short HEAD)
BIN=/opt/lel/current/bin
D=/srv/lel/bbp
TOY=/srv/lel/toy
LIBEXEC=/usr/local/libexec
STATE=/var/lib/mill-board
UNIT=/etc/systemd/system/mill-board.service
MARKER_DIR=/var/lib/lel-deploy   # root-owned: the board's own user cannot forge the fixture record
MARKER=$MARKER_DIR/1092-fixture.json
LEL_HOST=${LEL_HOST:-baileyai}
R="sudo -H -u orch-runner"   # orch-runner cannot enter baileyrd's directories: every call below runs from /
# shellcheck source=1092-board-lib.sh
. "$HERE/1092-board-lib.sh"
TASK=$FIXTURE_TASK

die() { echo "STOP: $*" >&2; exit 1; }
cd /
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
CHANGED=0
# put SRC DEST MODE: install SRC as root-owned DEST only if it differs; records that something changed.
put() {
  if sudo cmp -s "$1" "$2"; then return 0; fi
  sudo install -o root -g root -m "$3" "$1" "$2"
  CHANGED=1
  echo "installed $2"
}

# ---- 0. Pre-flight: read-only. Nothing below changes the host until all of it passes.
echo "=== 0. pre-flight"
command -v jq >/dev/null || die "jq is not installed"
host_check "$(hostname)" "$LEL_HOST" || die "wrong host"
[ -x "$BIN/bbp" ] || die "$BIN/bbp missing: run 1092-deploy.sh first"
[ "$(basename "$(readlink -f /opt/lel/current)")" = "$SHA" ] \
  || die "/opt/lel/current is not this checkout ($SHA): run 1092-deploy.sh again first"
$R test -d "$D" || die "$D missing: run 1092-deploy.sh first"
if id mill-board >/dev/null 2>&1; then
  [ "$(getent passwd mill-board | cut -d: -f6)" = "$STATE" ] || die "user mill-board exists with another home: not ours"
  if id -nG mill-board | tr ' ' '\n' | grep -qx orch-runner; then die "mill-board is in the orch-runner group"; fi
fi
if sudo test -e "$STATE"; then
  [ "$(sudo stat -c %U "$STATE")" = mill-board ] || die "$STATE is not owned by mill-board: not ours"
  for f in board.env board.db; do
    sudo test ! -e "$STATE/$f" || [ "$(sudo stat -c %U "$STATE/$f")" = mill-board ] || die "$STATE/$f is not owned by mill-board"
  done
fi
if sudo test -f "$STATE/board.env"; then
  sudo cat "$STATE/board.env" | env_check "$STATE" || die "$STATE/board.env is not this inbox board's; it is left as it is"
  HTTP=$(sudo cat "$STATE/board.env" | env_value MILL_BOARD_HTTP)
fi
HTTP=${HTTP:-127.0.0.1:8787}
loopback_check "$HTTP" || die "listener"
unit_check "$UNIT" "$STATE" || die "mill-board.service is not this inbox board's"
# What systemd would actually run, every drop-in directory included. A failed inspection stops the script.
EXPECTED=$(unit_expected "$UNIT" "$STATE" "$BIN" "$LIBEXEC" "$D")
unit_show() { systemctl show -p "$(tr -s ' \n' ',' <<<"$UNIT_PROPS" | sed 's/,$//')" mill-board; }
SHOW=$(unit_show) || die "cannot inspect mill-board.service"
unit_effective_check "$EXPECTED" 0 <<<"$SHOW" || die "mill-board.service is not the approved unit; nothing changed"
SVC=$(systemctl show -p ActiveState,User,MainPID mill-board) || die "cannot inspect mill-board.service"
ACTIVE=$(env_value ActiveState <<<"$SVC")
MAIN=$(env_value MainPID <<<"$SVC")
SS=$(sudo ss -Hltnp "( sport = :${HTTP##*:} )") || die "cannot list the listeners on port ${HTTP##*:}"
service_check "$ACTIVE" "$(env_value User <<<"$SVC")" "$MAIN" <<<"$SS" \
  || die "the board's service or port is not this inbox board's"
fixture_marker() {   # the recorded fixture, empty when there is none; a marker that cannot be read stops
  sudo test -e "$MARKER" || return 0
  [ "$(sudo stat -c %U:%G "$MARKER")" = root:root ] || die "$MARKER is not root-owned"
  sudo cat "$MARKER" || die "cannot read $MARKER"
}
task_state() {   # exists | absent; any other bbp failure is a failed lookup
  local err rc=0
  err=$($R "$BIN/bbp" card --dir "$D" --task "$TASK" 2>&1 >/dev/null) || rc=$?
  lookup_state "$rc" "$err"
}
FIX=$(fixture_marker)
TSTATE=$(task_state) || die "cannot tell whether $TASK exists"
EXISTS=0; [ "$TSTATE" = absent ] || EXISTS=1
PLAN=$(fixture_plan "$FIX" "$EXISTS") || die "the fixture task is not this script's"
echo "host $(hostname), board $HTTP, fixture $TASK: $PLAN"

# ---- 1. Build rusty_mill_board into the current release (beside bbp, same commit)
echo "=== 1. build rusty_mill_board"
( cd "$REPO" && cargo build --release -p rusty_mill_board )
put "$REPO/target/release/rusty_mill_board" "/opt/lel/$SHA/bin/rusty_mill_board" 0755
sudo restorecon -R /opt/lel

# ---- 2. The mill-board user and its state (D132): not in orch-runner's group
echo "=== 2. mill-board user"
id mill-board >/dev/null 2>&1 \
  || { sudo useradd --system --no-create-home --home-dir "$STATE" --shell /usr/sbin/nologin mill-board; CHANGED=1; }
sudo install -d -o mill-board -g mill-board -m 0700 "$STATE" "$STATE/settings"
sudo install -d -o root -g root -m 0755 "$MARKER_DIR"

# ---- 3. board.env: a separate inbox board with its own SQLite file (D175; rusty_tick is retired). Created once
# from the example and checked before it is installed; an existing one was checked in step 0 and is not edited.
echo "=== 3. board.env"
if ! sudo test -f "$STATE/board.env"; then
  sed -e '/^MILL_BOARD_TICK_URL=/d' -e '/^RUSTY_TICK_TOKEN=/d' -e '/^MILL_BOARD_DB=/d' \
    -e "s|^MILL_BOARD_SETTINGS_DIR=.*|MILL_BOARD_SETTINGS_DIR=$STATE/settings|" \
    "$REPO/crates/apps/rusty_mill_board/deploy/board.env.example" >"$TMP/board.env"
  echo "MILL_BOARD_DB=$STATE/board.db" >>"$TMP/board.env"
  env_check "$STATE" <"$TMP/board.env" || die "the generated board.env fails its own check"
  sudo install -o mill-board -g mill-board -m 0600 "$TMP/board.env" "$STATE/board.env"
  CHANGED=1
  echo "created $STATE/board.env"
fi
echo "db: $STATE/board.db   board: $HTTP"

# ---- 4. The bbp guard, its wrapper and the sudoers rule (layout "Directories"; the guard verbatim)
echo "=== 4. guard and sudoers"
cat >"$TMP/guard" <<'EOF'
#!/bin/sh
# Runs as orch-runner via sudo. Allows: bbp inbox|human --dir /srv/lel/bbp <args without another --dir>.
set -eu
case "${1-}" in inbox|human) ;; *) echo "mill-board-bbp-guard: verb refused" >&2; exit 64 ;; esac
if [ "${2-}" != --dir ] || [ "${3-}" != /srv/lel/bbp ]; then echo "mill-board-bbp-guard: --dir refused" >&2; exit 64; fi
verb=$1; shift 3
skip=0
for a in "$@"; do   # walks the args the way bbp's parser does: every --key takes the next token as its value
  if [ "$skip" = 1 ]; then skip=0; continue; fi
  case "$a" in
    --dir) echo "mill-board-bbp-guard: second --dir refused" >&2; exit 64 ;;
    --*) skip=1 ;;
  esac
done
exec /opt/lel/current/bin/bbp "$verb" --dir /srv/lel/bbp "$@"
EOF
printf '#!/bin/sh\nexec sudo -n -u orch-runner %s/mill-board-bbp-guard "$@"\n' "$LIBEXEC" >"$TMP/wrapper"
printf 'mill-board ALL=(orch-runner) NOPASSWD: %s/mill-board-bbp-guard\n' "$LIBEXEC" >"$TMP/sudoers"
sudo visudo -cf "$TMP/sudoers"
put "$TMP/guard" "$LIBEXEC/mill-board-bbp-guard" 0755
put "$TMP/wrapper" "$LIBEXEC/mill-board-bbp" 0755
put "$TMP/sudoers" /etc/sudoers.d/mill-board 0440
sudo restorecon "$LIBEXEC/mill-board-bbp-guard" "$LIBEXEC/mill-board-bbp" /etc/sudoers.d/mill-board

# ---- 5. systemd: the shipped unit (if none) plus the drop-in. Each list setting is cleared first.
echo "=== 5. systemd"
sudo test -f "$UNIT" || put "$REPO/crates/apps/rusty_mill_board/deploy/mill-board.service" "$UNIT" 0644
sudo install -d -m 0755 "$UNIT.d"
# -/run/sudo: the layout's fallback for ProtectSystem=strict leaving sudo's timestamp dir read-only; '-' skips it if absent.
RW="$STATE $D -/run/sudo"
cat >"$TMP/lel.conf" <<EOF
[Service]
User=mill-board
EnvironmentFile=
EnvironmentFile=$STATE/board.env
ExecStart=
ExecStart=$BIN/rusty_mill_board
ReadWritePaths=
ReadWritePaths=$RW
NoNewPrivileges=false
Environment=MILL_BOARD_BBP_BIN=$LIBEXEC/mill-board-bbp MILL_BOARD_BBP_ROOT=$D
EOF
put "$TMP/lel.conf" "$UNIT.d/lel.conf" 0644
sudo systemctl daemon-reload
SHOW=$(unit_show) || die "cannot inspect mill-board.service"
unit_effective_check "$EXPECTED" 1 <<<"$SHOW" || die "the installed unit is not the approved unit; not started"
sudo systemctl enable mill-board >/dev/null
if [ "$CHANGED" = 1 ] || [ "$ACTIVE" != active ]; then
  sudo systemctl restart mill-board
else
  echo "nothing changed; mill-board not restarted"
fi

# ---- 6. Verification step 1: health
echo "=== 6. step 1: health"
for _ in $(seq 1 30); do curl -fsS -m 2 "http://$HTTP/health" >/dev/null 2>&1 && break; sleep 1; done
curl -fsS -m 2 "http://$HTTP/health" || { sudo journalctl -u mill-board -n 30 --no-pager; die "board not healthy"; }
echo
# Whoever answered is the approved service: still the approved unit, active, and the only owner of the port.
SHOW=$(unit_show) || die "cannot inspect mill-board.service"
unit_effective_check "$EXPECTED" 1 <<<"$SHOW" || die "mill-board.service changed after start"
SVC=$(systemctl show -p ActiveState,User,MainPID mill-board) || die "cannot inspect mill-board.service"
SS=$(sudo ss -Hltnp "( sport = :${HTTP##*:} )") || die "cannot list the listeners on port ${HTTP##*:}"
service_check "$(env_value ActiveState <<<"$SVC")" "$(env_value User <<<"$SVC")" "$(env_value MainPID <<<"$SVC")" 1 <<<"$SS" \
  || die "the board's port is not held by mill-board.service; no fixture request sent"
echo "ok: port ${HTTP##*:} is held by mill-board.service (pid $(env_value MainPID <<<"$SVC")) only"

# ---- 7. Verification step 4 on the script-owned fixture (D146's A2 only; no lens run): a card on this board
# linked to it, inbox_list, inbox_act (dry run, then reject) on that exact row. The marker records each stage, so a
# rerun resumes, and a verified fixture is a no-op. The admin token is the fresh board's first-admin.token, or
# MB_TOKEN; it stays in a variable and a header file descriptor, never in argv or the output.
echo "=== 7. step 4: inbox ($PLAN)"
hook_marker_read()  { fixture_marker; }
hook_marker_write() { printf '%s\n' "$1" >"$TMP/marker" && sudo install -o root -g root -m 0644 "$TMP/marker" "$MARKER"; }
hook_task_state()   { task_state; }
hook_bbp_open()     { $R "$BIN/bbp" inbox --dir "$D" --task "$TASK" --json yes | jq length; }
hook_open() {   # the toy task (tree from 1166-smoke-run.sh); fails if any input is missing or bbp refuses
  local f
  for f in repo brief.md roster.json profiles.json holdout-profiles.json holdout; do
    $R test -e "$TOY/$f" || { echo "$TOY/$f missing: the toy tree comes from 1166-smoke-run.sh" >&2; return 1; }
  done
  $R "$BIN/bbp" open --mode claims --dir "$D" --task "$TASK" --repo "$TOY/repo" \
    --brief "$TOY/brief.md" --roster "$TOY/roster.json" --profiles "$TOY/profiles.json" \
    --holdout-profiles "$TOY/holdout-profiles.json" --holdout-root "$TOY/holdout" >&2
}
hook_assign() { $R "$BIN/bbp" assign --dir "$D" --task "$TASK" --role "$1" --principal "$1" --vendor local >&2; }
hook_seed_a2() {   # prints A2's id; fails if the brief id or the seed script fails
  local brief out
  brief=$($R "$BIN/bbp" card --dir "$D" --task "$TASK" | jq -er .brief) || return 1
  out=$(bash "$HERE/1166-seed-inbox.sh" "$D" "$TASK" "$brief" "$BIN") || return 1
  sed -n 's/^A2=//p' <<<"$out"
}
if [ "$PLAN" != "done" ]; then
  TOKEN=${MB_TOKEN:-$(sudo cat "$STATE/settings/first-admin.token" 2>/dev/null || true)}
  [ -n "$TOKEN" ] || die "no admin token: set MB_TOKEN to an admin token of this board, then rerun"
fi
hook_api() {   # hook_api OP JSON -> the result body
  curl -fsS -m 30 -H @<(printf 'Authorization: Bearer %s\n' "$TOKEN") \
    -H 'Content-Type: application/json' -d "$2" "http://$HTTP/api/$1"
}
fixture_verify || die "step 4 did not pass; the marker $MARKER records how far it got"

# ---- 8. Verification step 5: as mill-board under the unit's sandbox
echo "=== 8. step 5: guard"
as_board() {
  sudo systemd-run --quiet --wait --pipe --collect -p User=mill-board -p NoNewPrivileges=no \
    -p ProtectSystem=strict -p ProtectHome=read-only -p PrivateTmp=yes -p "ReadWritePaths=$RW" "$@"
}
as_board "$LIBEXEC/mill-board-bbp" inbox --dir "$D" --task "$TASK" --json yes | jq -c 'length as $n | "inbox rows: \($n)"'
refused() {   # refused PATTERN CMD...: CMD fails and prints the expected denial; any other failure is not a pass
  local want=$1 out; shift
  if out=$(as_board "$@" 2>&1); then echo "FAIL: allowed: $*"; echo "$out"; exit 1; fi
  grep -qiE "$want" <<<"$out" || { echo "FAIL: $* failed without the expected denial ($want):"; echo "$out"; exit 1; }
  echo "ok refused: $* -> $(grep -iE "$want" <<<"$out" | tail -n 1)"
}
refused 'mill-board-bbp-guard: second --dir refused' "$LIBEXEC/mill-board-bbp" inbox --dir "$D" --dir /tmp/x --task "$TASK"
refused 'mill-board-bbp-guard: --dir refused' "$LIBEXEC/mill-board-bbp" inbox --dir /tmp/x --task "$TASK"
refused 'mill-board-bbp-guard: verb refused' "$LIBEXEC/mill-board-bbp" open --dir "$D"
# Through a shell, as the wrapper does: systemd (init_t) may not exec sudo directly under SELinux, which
# would refuse the call before sudoers is consulted and prove nothing about the rule. Only sudo's own
# policy refusal counts.
refused '(^sudo: a password is required|sorry, user mill-board is not allowed)' \
  /bin/sh -c "sudo -n -u orch-runner /bin/cat $D/master"

echo "=== done. Steps 1, 4 and 5 passed."
if sudo test -f "$STATE/settings/first-admin.token"; then
  echo "This board's admin token is in $STATE/settings/first-admin.token: read it (sudo cat), keep it, then delete the file."
fi
echo "--- AVC denials (none expected)"
sudo ausearch -m avc -ts recent 2>/dev/null | tail -5 || true
