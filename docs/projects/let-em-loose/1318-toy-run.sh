#!/bin/bash
# #1318 D167 toy-claim run on BaileyAi: the ONE approved paid run (2 builders, 45-minute wall, one claim).
# Defaults: D197. Prompt and record: 1318-toy-run.md. No lens run; the only model calls are the two builders'.
#
# Run from a clean checkout of merged main, as baileyrd (not root), inside tmux so a dropped terminal can't
# stop the read-back (the builders themselves are detached and keep running either way):
#   tmux new -s toy 'bash docs/projects/let-em-loose/1318-toy-run.sh; exec bash'

# Everything lives in ~/toy-1318 (TOY_TASK=toy-1318b for a second try). It never uses sudo: when the binaries
# are missing from /opt/lel/<sha>/bin it prints the install lines for you and stops. Nothing under /srv/lel,
# /opt/lel/current or the inbox board changes.
set -euo pipefail
unset CDPATH   # a set CDPATH makes `cd` print the directory
# Builders use the subscription login only (D165): no API key reaches them.
unset ANTHROPIC_API_KEY ANTHROPIC_AUTH_TOKEN

# Its own variable, not TASK, so an exported TASK can't point the run (and its cleanup) at another dir.
TASK=${TOY_TASK:-toy-1318}
[[ $TASK =~ ^toy-1318[a-z0-9-]*$ ]] || { echo "STOP: TOY_TASK must start toy-1318 (letters, digits, -)" >&2; exit 1; }
T=$HOME/$TASK
MARK=$T/.lel-toy-run                        # only a dir holding this mark is ever removed
WALL=2700                                   # D167: 45 minutes per builder
UNIT=lel-$TASK-mux                          # the run's own mux server, a delegated user unit
REPO_ROOT=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
HERE=$REPO_ROOT/docs/projects/let-em-loose
SHA=$(git -C "$REPO_ROOT" rev-parse --short=12 HEAD)
BIN=/opt/lel/$SHA/bin
D=$T/bbp; STATE=$T/state; SLOTS=$T/slots; SOCK=$T/mux.sock; OUT=$T/out; CFG=$T/lens.json

die() { echo "STOP: $*" >&2; exit 1; }
. "$HERE/1318-toy-verdict.sh"   # toy_verdict: the step 7 session check, fixture-tested in 1318-toy-verdict.test.sh

# ---- 0. Pre-flight: read-only checks, in the order that costs least
echo "=== 0. pre-flight $(date -u +%FT%TZ), commit $SHA"
[ "$(id -u)" != 0 ] || die "run as baileyrd, not root"
[ "$(hostname -s | tr A-Z a-z)" = "${LEL_HOST:-baileyai}" ] || die "not on BaileyAi (set LEL_HOST if its short name differs)"
for c in jq python3 git flock systemd-run systemctl cargo npm node claude claude-code-acp; do
  command -v "$c" >/dev/null || die "$c is not on PATH"
done
[ -z "$(git -C "$REPO_ROOT" status --porcelain)" ] || die "the checkout has local changes"
git -C "$REPO_ROOT" fetch -q origin main
git -C "$REPO_ROOT" merge-base --is-ancestor HEAD origin/main || die "HEAD $SHA is not on origin/main (D197: run merged code only)"
[ -f "$REPO_ROOT/crates/apps/rusty_orch/crates/rusty_orch/src/builder/e2e.rs" ] || die "#1318 S6 is not merged at $SHA"
grep -qw landlock /sys/kernel/security/lsm || die "Landlock is not active"
[ ! -e "$D" ] || die "$D exists: a run already opened this task. Rerun with TOY_TASK=toy-1318b"
[ ! -e "$T" ] || [ -f "$MARK" ] || die "$T exists and this script didn't make it: move it, or use another TOY_TASK"

# ---- 1. Binaries from this sha (built here; installing needs sudo, so it is printed, never run)
echo "=== 1. binaries in $BIN"
NEED="bbp rusty_orch lel-jail rusty-mux-server rusty-mux-ctl"
missing=; for b in $NEED; do [ -x "$BIN/$b" ] || missing=1; done
if [ -n "$missing" ]; then
  (cd "$REPO_ROOT" && cargo build --release --locked -p rusty_bbp_host -p rusty_orch -p orch-jail -p rusty_mux_server -p rusty_mux_ctl)
  cat <<EOF

Built. Install them (this does not touch /opt/lel/current), then run this script again:
  sudo install -d -m 0755 $BIN
  sudo install -m 0755 $(for b in $NEED; do printf '%s ' "$REPO_ROOT/target/release/$b"; done)$BIN/
  sudo restorecon -R /opt/lel/$SHA
EOF
  exit 0
fi

# ---- 2. The no-overage assertion and the login (D167): fail closed before anything is made
echo "=== 2. assertion and login"
# Ours (the mark) and unpaid (no task dir, checked above), so it is safe to start clean.
[ ! -e "$T" ] || rm -rf "$T"
mkdir -p "$T" "$OUT"
touch "$MARK"
LOG=$OUT/script-$(date -u +%Y%m%dT%H%M%SZ).log
exec > >(tee -a "$LOG") 2>&1
echo "log: $LOG"
cat > "$T/p" <<'EOF'
Builder run: no lens prompt is used.
EOF
cat > "$CFG" <<EOF
{
  "providers": {
    "claude": {"kind": "claude-cli", "model": "opus", "context_tokens": 200000,
               "price": {"input": 0, "output": 0, "cached_input": 0}}
  },
  "ceilings": {"experiment_usd": 0, "task_usd": 0, "task_calls": 2},
  "builders": {"builders_per_claim": 2, "max_live_builders": 2, "builder_wall_secs": $WALL},
  "lenses": [
    {"id": "architect", "prompt": "p", "domains": ["*"], "may_post": {"claim": true},
     "models": {"routine": ["claude"], "contested": ["claude"]}}
  ]
}
EOF
CHECK=$("$BIN/rusty_orch" lens check "$CFG")
echo "$CHECK" | sed 's/[^[:space:]]*@[^[:space:]]*/<account>/g'   # the log is sent back: no email in it
ACCOUNT=$(sed -n 's/^claude[[:space:]].*ready if logged in as \([^[:space:]]*\).*/\1/p' <<<"$CHECK")
[ -n "$ACCOUNT" ] || die "no valid Claude no-overage assertion in /etc/let-em-loose/assertions.json (it lapses 2026-12-25)"
claude auth status 2>&1 | grep -qF "$ACCOUNT" || die "claude is not logged in as the asserted account"
echo "claude: asserted and logged in"

# Read roots for claude-code-acp and node when they live outside the system roots (the jail reads only those).
ROOTS=()
for p in "$(npm prefix -g)" "$(dirname "$(dirname "$(readlink -f "$(command -v node)")")")"; do
  case "$p" in ""|/usr|/usr/*|/bin|/lib|/lib64) ;; *) ROOTS+=("$p");; esac
done
READ=(); for p in "${ROOTS[@]}"; do READ+=(--read "$p"); done
ACP_VERSION=$(timeout 30 npm ls -g --depth=0 </dev/null 2>/dev/null | grep -o 'claude-code-acp@[^ ]*' || echo unknown)
echo "$ACP_VERSION; node $(node --version); extra read roots: ${ROOTS[*]:-none}" | tee "$OUT/acp-version.txt"

# ---- 3. The run's own mux server in a delegated user unit, so a builder's cgroup can be made below it
echo "=== 3. mux server ($UNIT)"
# The unit gets the user manager's environment, not this script's: no API key may be there either.
! systemctl --user show-environment | grep -q '^ANTHROPIC_' \
  || die "the systemd user manager has an ANTHROPIC_ variable: systemctl --user unset-environment it first"
systemctl --user is-active -q "$UNIT" && die "$UNIT is already running: stop it first (systemctl --user stop $UNIT)"
# Until the builders start, any STOP also stops the unit; after that the builders need it.
trap 'systemctl --user stop "$UNIT" 2>/dev/null || true' EXIT
systemd-run --user --quiet --unit "$UNIT" -p Delegate=yes --setenv=HOME="$HOME" \
  "$BIN/rusty-mux-server" --no-state "$SOCK"
for _ in $(seq 100); do [ -S "$SOCK" ] && break; sleep 0.1; done
[ -S "$SOCK" ] || die "the mux server never bound $SOCK: journalctl --user -u $UNIT"
CG=/sys/fs/cgroup$(systemctl --user show -P ControlGroup "$UNIT")
[ -w "$CG/cgroup.procs" ] || die "$CG is not delegated to $(id -un)"

# ---- 4. No-cost handshake: claude-code-acp starts in the jail and answers initialize and session/new.
# No prompt is sent, so no model is asked. A failure here stops the run before any paid call.
echo "=== 4. jailed handshake (no prompt)"
if ! systemd-run --user --quiet --scope -p Delegate=yes \
  python3 -I "$HERE/1318-acp-handshake.py" "$BIN/lel-jail" "$T/acp-check" "${ROOTS[@]}" \
  | tee "$OUT/handshake.txt"; then
  die "the jailed handshake failed: see $OUT/handshake.txt"
fi
grep -q '^handshake ok$' "$OUT/handshake.txt" || die "the jailed handshake failed: see $OUT/handshake.txt"
rm -rf "$T/acp-check"

# Everything free has passed. Nothing is made for the task until the paid run is confirmed.
read -r -p "Type RUN to make the task and start the two paid builders (anything else stops here): " GO || GO=
[ "$GO" = RUN ] || die "not confirmed: nothing was made and nothing paid ran"

# ---- 5. Toy repo, task and claim (D197 item 3)
echo "=== 5. toy repo, task, claim"
mkdir -p "$T/repo/tests" "$STATE" "$SLOTS"
printf 'agents=2 validators=1\n' > "$SLOTS/capacity"
cat > "$T/repo/sweep" <<'EOF'
#!/bin/sh
echo "usage: sweep --dry-run DIR DAYS" >&2
exit 2
EOF
cat > "$T/repo/tests/usage.sh" <<'EOF'
#!/bin/sh
# With no arguments, sweep prints usage and exits 2.
./sweep >/dev/null 2>&1
[ $? = 2 ]
EOF
chmod 0755 "$T/repo/sweep" "$T/repo/tests/usage.sh"
git -C "$T/repo" init -q
git -C "$T/repo" add -A
git -C "$T/repo" -c user.name=toy -c user.email=toy@invalid commit -q -m "toy: stub sweep and usage test"
BASE=$(git -C "$T/repo" rev-parse HEAD)
cat > "$T/brief.md" <<'EOF'
# sweep --dry-run (toy, #1318)

`sweep --dry-run DIR DAYS` prints `would delete <path>` for each regular file directly inside DIR whose
modification time is more than DAYS days old, and deletes nothing. It never follows a symlink or enters a
subdirectory. DAYS must be a positive whole number, otherwise it prints usage and exits 2. Shell only.
EOF
cat > "$T/profiles.json" <<'EOF'
{
  "profiles": [{"name": "test", "program": "/bin/sh", "args": ["-c", "sh tests/usage.sh"]}],
  "read_roots": ["/bin", "/usr", "/lib", "/lib64"],
  "env": [["PATH", "/usr/bin:/bin"]],
  "limits": {"cpu_secs": 300, "wall_secs": 600, "memory_bytes": 4294967296, "file_bytes": 1073741824, "open_files": 1024, "processes": 4096},
  "tree": "sha256(git write-tree id)",
  "test_paths": ["tests/**"],
  "erosion": "off"
}
EOF
role() { printf '{"id":"%s","turn_order":%s,"post":["Claim","Evidence"],"write":["Diff","Candidate","Source"],"read_artifacts":["Brief","Diff","Candidate","TestReport","Log","Source"],"read_messages":"All","verdict_at":[],"final_decision":false,"consultable":false}' "$1" "$2"; }
printf '[%s,%s]\n' "$(role claude-builder 0)" "$(role architect 1)" > "$T/roster.json"
OPENED=$("$BIN/bbp" open --mode claims --dir "$D" --task "$TASK" --repo "$T/repo" --brief "$T/brief.md" \
  --roster "$T/roster.json" --profiles "$T/profiles.json")
echo "$OPENED"
case "$OPENED" in *ejected*) die "bbp open was refused";; esac
"$BIN/bbp" assign --dir "$D" --task "$TASK" --role claude-builder --principal claude-builder --vendor claude
"$BIN/bbp" assign --dir "$D" --task "$TASK" --role architect --principal architect --vendor fixture
"$BIN/bbp" card --dir "$D" --task "$TASK" > "$OUT/card-before.json"
BRIEF=$(jq -r .brief "$OUT/card-before.json")
[[ $BRIEF =~ ^[0-9]+$ ]] || die "brief id is not a number: '$BRIEF'"
TEXT='Build `sweep --dry-run DIR DAYS` in this repo as the brief says, with a test under tests/ for each rule it states.'
CLAIM=$(printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"fixture","version":"0"}}}' \
  '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  "$(jq -cn --arg t "$TEXT" --argjson b "$BRIEF" '{jsonrpc:"2.0",id:2,method:"tools/call",params:{name:"post",arguments:{op:"toy-claim",kind:"claim",text:$t,domains:["sweep"],stakes:"low",reversible:true,deps:[],refs:["art:\($b)"]}}}')" \
  | env BBP_DIR="$D" BBP_TASK="$TASK" BBP_PRINCIPAL=architect BBP_SESSION=yes "$BIN/bbp" mcp \
  | tee "$OUT/claim.out" | jq -er 'select(.id==2).result.content[0].text|fromjson|.Posted')
[[ $CLAIM =~ ^[0-9]+$ ]] || die "the claim was refused: see $OUT/claim.out"
echo "claim $CLAIM"
SCOPE=$("$BIN/bbp" cgroup-scope --dir "$D")
mkdir "$CG/$SCOPE"
"$BIN/bbp" verify --dir "$D" --task "$TASK" | tee "$OUT/verify-before.txt"
sessions() { "$BIN/rusty_orch" builder sessions "$CFG" --dir "$D" --task "$TASK" --state "$STATE" --claim "$CLAIM"; }
# Step 7 trusts only the persisted sessions, so the reader must work before anything is spent.
before=$(sessions) || die "rusty_orch builder sessions failed on the empty store: no way to check the run"
[ -z "$before" ] || die "the store already holds sessions for claim $CLAIM: $before"

# ---- 6. The paid part: two concurrent builders (D197 item 4), detached so a closed terminal can't stop them
echo "=== 6. two builders, ${WALL}s wall each, from $(date -u +%FT%TZ)"
launch() {
  setsid "$BIN/rusty_orch" builder run "$CFG" --bbp "$BIN/bbp" --dir "$D" --task "$TASK" --claim "$CLAIM" \
    --state "$STATE" --repo "$T/repo" --base "$BASE" --cgroup-parent "$CG/$SCOPE" --slots "$SLOTS" \
    --mux-ctl "$BIN/rusty-mux-ctl" --mux-socket "$SOCK" --jail "$BIN/lel-jail" "${READ[@]}" \
    >"$OUT/row-$1.json" 2>"$OUT/builder-$1.err" </dev/null &
  echo $!
}
trap - EXIT
P1=$(launch 1); sleep 2; P2=$(launch 2)
echo "builders: pids $P1 $P2. Watch: $BIN/rusty-mux-ctl --socket $SOCK list (session lel-$TASK, window builders)"
while kill -0 "$P1" 2>/dev/null || kill -0 "$P2" 2>/dev/null; do
  echo "$(date -u +%T)Z running: $(for p in $P1 $P2; do kill -0 "$p" 2>/dev/null && printf '%s ' "$p"; done)"
  sleep 60
done

# ---- 7. Read back and the launch-pass checks (D197 item 6)
echo "=== 7. read back $(date -u +%FT%TZ)"
"$BIN/bbp" card   --dir "$D" --task "$TASK"                  | tee "$OUT/card-after.json"
"$BIN/bbp" why    --dir "$D" --task "$TASK" --claim "$CLAIM" | tee "$OUT/why-claim.json"
"$BIN/bbp" verify --dir "$D" --task "$TASK"                  | tee "$OUT/verify-after.txt"
cp -r "$STATE/builders" "$OUT/" 2>/dev/null || true
fail=0
# The stdout rows alone prove nothing: a row is printed even when its final save failed.
sessions >"$OUT/sessions.jsonl" 2>"$OUT/sessions.err" || { echo "FAIL: rusty_orch builder sessions failed: see $OUT/sessions.err"; fail=1; }
echo "persisted:"; cat "$OUT/sessions.jsonl"
toy_verdict "$CLAIM" "$OUT/row-1.json" "$OUT/row-2.json" "$OUT/sessions.jsonl" || fail=1
for n in 1 2; do
  row=$(cat "$OUT/row-$n.json"); echo "row $n: $row"
  m=$(jq -r .n <<<"$row" 2>/dev/null || echo "")
  [ -s "$STATE/builders/$CLAIM/$m/acp.log" ] || { echo "FAIL: builder $n has no acp.log"; fail=1; }
done
for c in "$CG/$SCOPE"/*/; do
  [ -d "$c" ] && grep -q 'populated 1' "$c/cgroup.events" && { echo "FAIL: $c is still populated"; fail=1; }
done
for l in "$SLOTS"/agent-*.lock; do
  [ -e "$l" ] && ! flock -n -x "$l" true && { echo "FAIL: $l is still held"; fail=1; }
done
systemctl --user stop "$UNIT" || true
echo "acp.logs: $OUT/builders/$CLAIM/*/acp.log; candidate (if any): jq .current $OUT/card-after.json"
cat <<EOF

=== Done. Send Claude $OUT (rows, sessions.jsonl, builder-*.err, builders/*/acp.log, card/why/verify,
handshake.txt, acp-version.txt, $LOG). Do not rerun: D167 approved one run.
EOF
if [ "$fail" = 0 ]; then
  echo "LAUNCH PASS: both sessions persisted as ended by D188's rule, matching stdout; cgroups empty; slots free."
else
  echo "LAUNCH FAIL: see the lines above. Nothing was retried."
  exit 1
fi
