#!/bin/bash
# #1092 Mill Board on BaileyAi with the decision inbox (D132): the `mill-board` user, the bbp guard,
# the sudoers rule and the systemd drop-in from 1092-deploy-layout.md ("Users", "systemd", "Decision"),
# then verification steps 1, 5 and 4 on a toy escalation. No model is called ($0).
# A separate inbox board on BaileyAi with its own SQLite file (Nano, 2026-11-25); the live board is untouched.
# Run from a checkout as baileyrd, after 1092-deploy.sh at the same commit:
#   bash docs/projects/let-em-loose/1092-deploy-board.sh
# Idempotent. Never prints board.env. Never touches /srv/lel/experiment, lel-codex or an existing task.
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
TASK=${TASK:-board-check}
R="sudo -H -u orch-runner"   # orch-runner cannot enter baileyrd's directories: every call below runs from /

die() { echo "STOP: $*" >&2; exit 1; }
cd /

# ---- 0. Pre-flight
echo "=== 0. pre-flight"
command -v jq >/dev/null || die "jq is not installed"
[ -x "$BIN/bbp" ] || die "$BIN/bbp missing: run 1092-deploy.sh first"
[ "$(basename "$(readlink -f /opt/lel/current)")" = "$SHA" ] \
  || die "/opt/lel/current is not this checkout ($SHA): run 1092-deploy.sh again first"
$R test -d "$D" || die "$D missing: run 1092-deploy.sh first"

# ---- 1. Build rusty_mill_board into the current release (beside bbp, same commit)
echo "=== 1. build rusty_mill_board"
( cd "$REPO" && cargo build --release -p rusty_mill_board )
sudo install -m 0755 "$REPO/target/release/rusty_mill_board" "/opt/lel/$SHA/bin/"
sudo restorecon -R /opt/lel

# ---- 2. The mill-board user and its state (D132): not in orch-runner's group
echo "=== 2. mill-board user"
id mill-board >/dev/null 2>&1 \
  || sudo useradd --system --no-create-home --home-dir "$STATE" --shell /usr/sbin/nologin mill-board
if id -nG mill-board | tr ' ' '\n' | grep -qx orch-runner; then die "mill-board is in the orch-runner group"; fi
sudo install -d -o mill-board -g mill-board -m 0700 "$STATE" "$STATE/settings"

# ---- 3. board.env: a separate inbox board with its own SQLite file (Nano, 2026-11-25: second board on
# BaileyAi; rusty_tick is retired). The live board elsewhere is not touched. Never printed.
echo "=== 3. board.env"
sudo test -f "$STATE/board.env" || sudo install -o mill-board -g mill-board -m 0600 \
  "$REPO/crates/apps/rusty_mill_board/deploy/board.env.example" "$STATE/board.env"
sudo sed -i -e '/^MILL_BOARD_TICK_URL=/d' -e '/^RUSTY_TICK_TOKEN=/d' -e '/^MILL_BOARD_DB=/d' \
  -e "s|^MILL_BOARD_SETTINGS_DIR=.*|MILL_BOARD_SETTINGS_DIR=$STATE/settings|" "$STATE/board.env"
echo "MILL_BOARD_DB=$STATE/board.db" | sudo tee -a "$STATE/board.env" >/dev/null
sudo chown -R mill-board:mill-board "$STATE"
HTTP=$(sudo sed -n 's/^MILL_BOARD_HTTP=//p' "$STATE/board.env" | tail -n 1)
HTTP=${HTTP:-127.0.0.1:8787}
echo "db: $STATE/board.db   board: $HTTP"

# ---- 4. The bbp guard, its wrapper and the sudoers rule (layout "Directories"; the guard verbatim)
echo "=== 4. guard and sudoers"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
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
sudo install -o root -g root -m 0755 "$TMP/guard" "$LIBEXEC/mill-board-bbp-guard"
sudo install -o root -g root -m 0755 "$TMP/wrapper" "$LIBEXEC/mill-board-bbp"
sudo install -o root -g root -m 0440 "$TMP/sudoers" /etc/sudoers.d/mill-board
sudo restorecon "$LIBEXEC/mill-board-bbp-guard" "$LIBEXEC/mill-board-bbp" /etc/sudoers.d/mill-board

# ---- 5. systemd: the shipped unit (if none) plus the drop-in. Each list setting is cleared first.
echo "=== 5. systemd"
sudo test -f "$UNIT" || sudo install -m 0644 "$REPO/crates/apps/rusty_mill_board/deploy/mill-board.service" "$UNIT"
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
sudo install -m 0644 "$TMP/lel.conf" "$UNIT.d/lel.conf"
sudo systemctl daemon-reload
sudo systemctl enable mill-board >/dev/null
sudo systemctl restart mill-board

# ---- 6. Verification step 1: health
echo "=== 6. step 1: health"
for _ in $(seq 1 30); do curl -fsS -m 2 "http://$HTTP/health" >/dev/null 2>&1 && break; sleep 1; done
curl -fsS -m 2 "http://$HTTP/health" || { sudo journalctl -u mill-board -n 30 --no-pager; die "board not healthy"; }
echo

# ---- 7. A toy task whose only escalation is the A2 fixture (D146), for step 4. No lens run.
echo "=== 7. task $TASK"
if ! $R "$BIN/bbp" card --dir "$D" --task "$TASK" >/dev/null 2>&1; then
  for f in repo brief.md roster.json profiles.json holdout-profiles.json holdout; do
    $R test -e "$TOY/$f" || die "$TOY/$f missing: the toy tree comes from 1166-smoke-run.sh"
  done
  $R "$BIN/bbp" open --mode claims --dir "$D" --task "$TASK" --repo "$TOY/repo" \
    --brief "$TOY/brief.md" --roster "$TOY/roster.json" --profiles "$TOY/profiles.json" \
    --holdout-profiles "$TOY/holdout-profiles.json" --holdout-root "$TOY/holdout"
  for r in seed architect adversary operator seed-s1 seed-s2; do   # the toy roster, as 1166-smoke-run.sh assigns it
    $R "$BIN/bbp" assign --dir "$D" --task "$TASK" --role "$r" --principal "$r" --vendor local
  done
  BRIEF=$($R "$BIN/bbp" card --dir "$D" --task "$TASK" | jq -r .brief)
  bash "$HERE/1166-seed-inbox.sh" "$D" "$TASK" "$BRIEF" "$BIN"
else
  echo "task $TASK exists; not seeded again"
fi

# ---- 8. Verification step 5: as mill-board under the unit's sandbox
echo "=== 8. step 5: guard"
as_board() {
  sudo systemd-run --quiet --wait --pipe --collect -p User=mill-board -p NoNewPrivileges=no \
    -p ProtectSystem=strict -p ProtectHome=read-only -p PrivateTmp=yes -p "ReadWritePaths=$RW" "$@"
}
as_board "$LIBEXEC/mill-board-bbp" inbox --dir "$D" --task "$TASK" --json yes | jq -c 'length as $n | "inbox rows: \($n)"'
refused() {   # the command must fail and print the guard's (or sudo's) refusal; bbp must not run
  local out
  if out=$(as_board "$@" 2>&1); then echo "FAIL: allowed: $*"; echo "$out"; exit 1; fi
  echo "ok refused: $* -> $(tail -n 1 <<<"$out")"
}
refused "$LIBEXEC/mill-board-bbp" inbox --dir "$D" --dir /tmp/x --task "$TASK"
refused "$LIBEXEC/mill-board-bbp" inbox --dir /tmp/x --task "$TASK"
refused "$LIBEXEC/mill-board-bbp" open --dir "$D"
refused sudo -n -u orch-runner /bin/cat "$D/master"

# ---- 9. Verification step 4: a card on this board linked to the task, inbox_list, inbox_act (dry run, then reject).
# The admin token is the fresh board's first-admin.token, or MB_TOKEN; it stays in a variable and a header file
# descriptor, never in argv or the output.
echo "=== 9. step 4: inbox"
TOKEN=${MB_TOKEN:-$(sudo cat "$STATE/settings/first-admin.token" 2>/dev/null || true)}
[ -n "$TOKEN" ] || die "no admin token: set MB_TOKEN to an admin token of this board, then rerun"
api() {   # api OP JSON -> the result body
  curl -fsS -m 30 -H @<(printf 'Authorization: Bearer %s\n' "$TOKEN") \
    -H 'Content-Type: application/json' -d "$2" "http://$HTTP/api/$1"
}
ROWS=$(api inbox_list '{}')
if ! jq -e --arg t "$TASK" 'any(.[]; .task==$t)' <<<"$ROWS" >/dev/null; then
  CARD=$(api card_add '{"title":"board-check: #1092 inbox verification","body":"Linked to BBP task board-check (A2 fixture, D146). Verification step 4 of 1092-deploy-layout.md."}' | jq -r '.card.number // .number')
  [[ $CARD =~ ^[0-9]+$ ]] || die "card_add returned no card number"
  api card_execution_link "{\"id\":\"$CARD\",\"execution\":\"$TASK\",\"goal\":\"$TASK\",\"bbp_task\":\"$TASK\",\"attempt\":\"1\",\"assignment\":\"$TASK\",\"generation\":1}" >/dev/null
  echo "card #$CARD linked to $TASK"
  ROWS=$(api inbox_list '{}')
fi
ROW=$(jq -c --arg t "$TASK" '[.[] | select(.task==$t)][0] // empty' <<<"$ROWS")
[ -n "$ROW" ] || die "inbox_list does not show $TASK; bbp says: $($R "$BIN/bbp" inbox --dir "$D" --task "$TASK" --json yes)"
echo "inbox_list: $(jq -c '{id, rev, reason, label, card}' <<<"$ROW")"
ACT=$(jq -c '{id, rev, action: "reject"}' <<<"$ROW")
echo "dry run: $(api inbox_act "$(jq -c '. + {dry_run: true}' <<<"$ACT")")"
echo "reject:  $(api inbox_act "$ACT")"
LEFT=$(api inbox_list '{}' | jq --arg t "$TASK" '[.[] | select(.task==$t)] | length')
BBP_LEFT=$($R "$BIN/bbp" inbox --dir "$D" --task "$TASK" --json yes | jq length)
[ "$LEFT" = 0 ] && [ "$BBP_LEFT" = 0 ] || die "escalation still open (board $LEFT, bbp $BBP_LEFT)"
echo "ok: inbox_list and bbp inbox are empty for $TASK"

echo "=== done. Steps 1, 4 and 5 passed."
if sudo test -f "$STATE/settings/first-admin.token"; then
  echo "This board's admin token is in $STATE/settings/first-admin.token: read it (sudo cat), keep it, then delete the file."
fi
echo "--- AVC denials (none expected)"
sudo ausearch -m avc -ts recent 2>/dev/null | tail -5 || true
