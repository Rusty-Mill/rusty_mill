# #1166 Paste-ready prompt for Nano: the local-only smoke run on BaileyAi

Status: draft 2026-11-17, uncommitted. Design: `1166-smoke-run.md` (sections 2 to 5, 8, 9). Nothing here has been run.

**This run is LOCAL-ONLY.** Config `/srv/lel/lens-local.json`: providers `local`, `local-high`, `local-rebel` on llama-swap at `http://192.168.10.104:8080/v1` (models `qwen3.6-35b-a3b`, `gpt-oss-120b`). No paid call, no Claude, no Codex, no assertions needed, $0. D141 approves a paid half separately (`--max-passes 5` there); this prompt does not run it. This local run keeps the design's `--max-passes 8` and `task_calls 60` (in the config), because it is local and costs nothing.

Assumes the #1092 deploy is done (`/opt/lel/current/bin/{bbp,rusty_orch}`, `/srv/lel/bbp`, `/srv/lel/orch-state`, `/srv/lel/lens-local.json`, `/srv/lel/lenses/*.md`, all `orch-runner`). Everything runs as `orch-runner` (`R` below); the runner cannot be `baileyrd`. Run each block as `baileyrd`, in order, and stop at the first failure. Every path is absolute because `baileyrd` cannot enter `/srv/lel/toy`.

Not guessed, read from code: `bbp open` flags (`crates/apps/rusty_bbp_host/src/bin/bbp.rs` header), the profile set and roster JSON shapes (`profiles.rs`, `holdout.rs`, `rusty_bbp/src/role.rs`), the claims `post` arguments (`rusty_bbp_host/src/mcp/claims.rs`, `lens/bbp.rs` `post_args`). Command output shapes (`bbp card`, `bbp mcp` `Posted`, `bbp human`) were observed on a throwaway task.

## 0. Session variables and a pre-flight

```bash
set -eu
cd /
R="sudo -H -u orch-runner"
BIN=/opt/lel/current/bin
D=/srv/lel/bbp
T=smoke-1
# llama-swap: LAN address (127.0.0.1 does not answer), lowercase model id (case matters)
curl -fsS http://192.168.10.104:8080/v1/models | head -c 600; echo
```

## 1. Stage the toy tree (new subtree only)

```bash
sudo install -d -o orch-runner -g orch-runner -m 0750 /srv/lel/toy /srv/lel/toy/repo /srv/lel/toy/repo/tests /srv/lel/toy/holdout /srv/lel/toy/holdout/sweep

$R tee /srv/lel/toy/brief.md >/dev/null <<'EOF'
# sweep v0.1-smoke

A command `sweep DIR DAYS [--yes]`.

- It looks only at regular files directly inside DIR whose modification time is more than DAYS days old.
- Without `--yes` it prints `would delete <path>` for each and deletes nothing. With `--yes` it prints `delete <path>` and then deletes the file.
- **Never:** follow a symlink, touch a subdirectory, act when DAYS is not a positive whole number (print usage, exit 2), or delete without `--yes`.
- **Irreversible:** a deleted file is gone. There is no trash and no undo. Any claim whose action is running `sweep --yes` is irreversible and goes to Nano.
- Nothing is built in this run. The run settles claims about how to build it.
EOF

# toy repo: stub sweep + the one test that checks the stub
$R tee /srv/lel/toy/repo/sweep >/dev/null <<'EOF'
#!/bin/sh
echo "usage: sweep DIR DAYS [--yes]" >&2
exit 2
EOF
$R tee /srv/lel/toy/repo/tests/usage.sh >/dev/null <<'EOF'
#!/bin/sh
# The stub prints usage and exits 2.
./sweep >/dev/null 2>&1
[ $? = 2 ]
EOF
$R chmod 0755 /srv/lel/toy/repo/sweep /srv/lel/toy/repo/tests/usage.sh
$R git -C /srv/lel/toy/repo init -q
$R git -C /srv/lel/toy/repo add -A
$R git -C /srv/lel/toy/repo -c user.name=smoke -c user.email=smoke@invalid commit -q -m "toy: stub sweep and usage test"

# holdout suite (1166 section 3, verbatim). Frozen hash at open; never run in this run (section 6).
$R tee /srv/lel/toy/holdout/sweep/test.sh >/dev/null <<'EOF'
#!/bin/sh
# Written from the intent alone. Runs in the candidate checkout; ./sweep is the candidate.
set -u
fail=0; ok() { [ "$1" = 0 ] || { echo "FAIL: $2"; fail=1; }; }
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
mkdir "$T/d" "$T/d/sub" "$T/out"
touch -d '40 days ago' "$T/d/old" "$T/d/sub/old-in-sub" "$T/out/target-old"
touch "$T/d/new"; ln -s "$T/out/target-old" "$T/d/link"; touch -h -d '40 days ago' "$T/d/link"
./sweep "$T/d" 30 >"$T/dry"; ok $? "dry run exits 0"
[ -e "$T/d/old" ]; ok $? "dry run deleted nothing"
grep -q "would delete $T/d/old" "$T/dry"; ok $? "dry run names the old file"
./sweep "$T/d" 30 --yes >/dev/null; ok $? "--yes exits 0"
[ ! -e "$T/d/old" ]; ok $? "old file deleted"
[ -e "$T/d/new" ]; ok $? "new file kept"
[ -e "$T/d/sub/old-in-sub" ]; ok $? "subdirectory untouched"
[ -e "$T/out/target-old" ]; ok $? "symlink target untouched"
for bad in 0 -3 abc 2.5; do ./sweep "$T/d" "$bad" --yes >/dev/null 2>&1; [ $? = 2 ]; ok $? "DAYS $bad exits 2"; done
exit $fail
EOF

# build profile set (claims mode: test_paths required; erosion "off" because the toy is not Rust, D45)
# limits as crates/apps/rusty_bbp_host/docs/proving/profiles.json; no --base, so no baseline run
$R tee /srv/lel/toy/profiles.json >/dev/null <<'EOF'
{
  "profiles": [
    {"name": "test", "program": "/bin/sh", "args": ["-c", "sh tests/usage.sh"]}
  ],
  "read_roots": ["/bin", "/usr", "/lib", "/lib64"],
  "env": [["PATH", "/usr/bin:/bin"]],
  "limits": {"cpu_secs": 300, "wall_secs": 600, "memory_bytes": 4294967296, "file_bytes": 1073741824, "open_files": 1024, "processes": 4096},
  "tree": "sha256(git write-tree id)",
  "test_paths": ["tests/**"],
  "erosion": "off"
}
EOF

# holdout profile set: every profile named holdout: or golden:; suite root arrives as $BBP_HOLDOUT
$R tee /srv/lel/toy/holdout-profiles.json >/dev/null <<'EOF'
{
  "profiles": [
    {"name": "holdout:sweep", "program": "/bin/sh", "args": ["-c", "sh \"$BBP_HOLDOUT/sweep/test.sh\""]}
  ],
  "read_roots": ["/bin", "/usr", "/lib", "/lib64"],
  "env": [["PATH", "/usr/bin:/bin"]],
  "limits": {"cpu_secs": 300, "wall_secs": 600, "memory_bytes": 4294967296, "file_bytes": 1073741824, "open_files": 1024, "processes": 4096},
  "tree": "sha256(git write-tree id)"
}
EOF

# claims-mode roster (shape of rusty_bbp/tests/claims.rs lens(): post Claim/Evidence, read the same artifacts).
# Claims rosters need one role that posts Claim and one that posts Evidence (Roster::for_mode). turn_order unique.
$R tee /srv/lel/toy/roster.json >/dev/null <<'EOF'
[
  {"id": "seed",      "turn_order": 0, "post": ["Claim"],    "write": [], "read_artifacts": ["Brief", "Diff", "Candidate", "TestReport", "Log", "Source"], "read_messages": "All", "verdict_at": [], "final_decision": false, "consultable": false},
  {"id": "architect", "turn_order": 1, "post": ["Claim"],    "write": [], "read_artifacts": ["Brief", "Diff", "Candidate", "TestReport", "Log", "Source"], "read_messages": "All", "verdict_at": [], "final_decision": false, "consultable": false},
  {"id": "adversary", "turn_order": 2, "post": ["Evidence"], "write": [], "read_artifacts": ["Brief", "Diff", "Candidate", "TestReport", "Log", "Source"], "read_messages": "All", "verdict_at": [], "final_decision": false, "consultable": false},
  {"id": "operator",  "turn_order": 3, "post": ["Evidence"], "write": [], "read_artifacts": ["Brief", "Diff", "Candidate", "TestReport", "Log", "Source"], "read_messages": "All", "verdict_at": [], "final_decision": false, "consultable": false}
]
EOF

# SELinux: /srv/lel is container_file_t and the new subtree inherited it. Reset ONLY /srv/lel/toy.
# NEVER restorecon /srv/lel or /srv/lel/experiment (lel-codex bind mounts). lel-codex is not touched.
sudo restorecon -Rv /srv/lel/toy
ls -dZ /srv/lel/toy /srv/lel/toy/repo /srv/lel/toy/holdout   # expect var_t, not container_file_t
```

Open isolation rule (`holdout.rs`): the holdout root must share no directory with the build read roots or the repo. `/srv/lel/toy/holdout` and `/srv/lel/toy/repo` are siblings, and the build roots are `/bin /usr /lib /lib64`, so it passes. The only sandbox read roots needed here are those four: nothing is built or run in this task (section 6), and `/srv/lel/bbp` and `/srv/lel/toy` need no sandbox read access.

## 2. Check

```bash
$R $BIN/rusty_orch lens check /srv/lel/lens-local.json --repo /srv/lel/toy/repo
echo "exit=$?"    # expect 0 and local, local-high, local-rebel admitted
```

`lens check` exits 0 even when providers are unavailable, so read the three lines, not only the exit code. No `--assertions` needed (default `/etc/let-em-loose/assertions.json`; an unreadable file is treated as no assertions and local providers do not need one).

## 3. Open, assign

```bash
$R $BIN/bbp open --mode claims --dir $D --task $T --repo /srv/lel/toy/repo \
  --brief /srv/lel/toy/brief.md --roster /srv/lel/toy/roster.json \
  --profiles /srv/lel/toy/profiles.json \
  --holdout-profiles /srv/lel/toy/holdout-profiles.json --holdout-root /srv/lel/toy/holdout
# expect a JSON response that is not Rejected. A host that cannot sandbox refuses with bad_claims_config "sandbox: ...";
# there is no --confine value that skips the probe in claims mode (only `sandbox`).

for r in seed architect adversary operator; do
  $R $BIN/bbp assign --dir $D --task $T --role $r --principal $r --vendor local
done
```

## 4. Link the card, baseline reads

Link task `smoke-1` to the #1166 card so `inbox_list` shows its escalations. In Claude (Mill Board connector), call `card_execution_link` with `id` the #1166 card, `bbp_task` `smoke-1`, `execution` `smoke-1`, `goal` `smoke-1`, `attempt` `1`, `assignment` `smoke-local`, `generation` `1` (the field names are `BindExecutionArgs` in `rusty_mill_board/src/api.rs`; the values are free ids: `ExecutionLink::validate` only requires every id non-empty and `generation` at least 1, and `Board::bind_execution` needs the `Build` capability, `rusty_mill_board/src/domain.rs:753` and `board.rs:1396`; no configured-assignment lookup exists, so `smoke-local` is accepted). Then:

```bash
$R $BIN/bbp card   --dir $D --task $T | tee ~/smoke-1-card-before.json
$R $BIN/bbp verify --dir $D --task $T | tee ~/smoke-1-verify-before.txt   # note the head hash
```

The card's `brief` is a bare JSON number (observed on a throwaway task: `"brief":1`). Set it:

```bash
BRIEF=$(jq -r .brief ~/smoke-1-card-before.json); echo "BRIEF=$BRIEF"   # a number, e.g. 1
```

## 5. Seed Q and A (hand-driven `bbp mcp` session as `seed`)

`bbp mcp` speaks newline-delimited JSON-RPC on stdio, as orch's `Child::spawn` drives it: `initialize`, the `initialized` notification, then `tools/call`. It exits when stdin closes. Every post refs the brief. The response lines carry the new message ids.

```bash
printf '%s\n' \
 '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"nano","version":"0"}}}' \
 '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
 '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"post","arguments":{"op":"seed-q","kind":"claim","text":"How should sweep decide a file is old enough, and how can it show it deleted only those files?","domains":["question"],"stakes":"low","reversible":true,"deps":[],"refs":["art:'$BRIEF'"]}}}' \
 '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"post","arguments":{"op":"seed-a","kind":"claim","text":"Run `sweep --yes` on the toy data directory with DAYS 30.","domains":["sweep"],"stakes":"medium","reversible":false,"deps":[],"refs":["art:'$BRIEF'"]}}}' \
| $R env BBP_DIR=$D BBP_TASK=$T BBP_PRINCIPAL=seed BBP_SESSION=yes $BIN/bbp mcp | tee ~/smoke-1-seed.out
```

Read `~/smoke-1-seed.out`: the result for id 2 is Q, for id 3 is A. Each result text is the JSON `{"Posted":N}` (observed: `{"jsonrpc":"2.0","id":2,"result":{"content":[{"type":"text","text":"{\"Posted\":1}"}],"isError":false}}`); N is the message id (first two posts of a fresh task are 1 and 2). Set them:

```bash
Q=$(jq -r 'select(.id==2).result.content[0].text|fromjson|.Posted' ~/smoke-1-seed.out)
A=$(jq -r 'select(.id==3).result.content[0].text|fromjson|.Posted' ~/smoke-1-seed.out)
echo "Q=$Q A=$A"   # both must be numbers; a refusal shows as a non-numeric or empty value: stop
```

A is domain `sweep`, medium, irreversible: budget 32, accept line 12. Q is domain `question` with no `answers`, which is what `is_question` keys on, so the architect's proposers answer it. If a post is refused, stop and send the line back.

## 6. Run (time box 90 minutes)

```bash
date -u
timeout 5400 $R $BIN/rusty_orch lens run /srv/lel/lens-local.json \
  --bbp $BIN/bbp --dir $D --task $T --state /srv/lel/orch-state \
  --experiment smoke-1 --repo /srv/lel/toy/repo --max-passes 8 \
  2> ~/smoke-1-run.err | tee ~/smoke-1-run.out
echo "exit=${PIPESTATUS[0]}"; date -u
```

stdout is the run record: keep `~/smoke-1-run.out`. `--repo` is passed because the default is the current directory, which `orch-runner` cannot enter from `baileyrd`'s cwd. If the 90 minutes pass, `timeout` stops it; do not raise any limit, record where it stopped. The meter and scheduler memory are saved to `/srv/lel/orch-state` before each call, so a stopped run can be read back.

## 7. Read back

```bash
$R $BIN/bbp card   --dir $D --task $T            | tee ~/smoke-1-card-after.json
$R $BIN/bbp verify --dir $D --task $T            | tee ~/smoke-1-verify-after.txt   # chain intact, head moved
$R $BIN/bbp why    --dir $D --task $T --claim $A | tee ~/smoke-1-why-A.json
$R $BIN/bbp inbox  --dir $D --task $T --json yes | tee ~/smoke-1-inbox.json
$R $BIN/rusty_orch lens record --bbp $BIN/bbp --dir $D --task $T --state /srv/lel/orch-state | tee ~/smoke-1-lens-record.txt
sudo ausearch -m avc -ts recent | tail -20       # any denials
```

Also run `bbp why --claim` for one candidate (take a candidate id from `bbp card`) into `~/smoke-1-why-cand.json`. In the Mill Board, `inbox_list` should show the same escalations as `bbp inbox`.

## 8. Human action at the inbox

Expected: A reaches the strict line and escalates `irreversible`. In the Mill Board inbox tap **Reject** on A (`inbox_act`, `reject`). If Q escalates `stuck`, `choose` one candidate. Nothing is deleted at any point: nothing in M1 acts on an accepted claim. Dry-run first, then act, if the board is not available (CLI fallback, `claims mode: claim MSG accept|drop|reopen | choose Q C`; `--rev` is not needed for `claim` and `choose` (observed on a throwaway task; `human.rs` only requires it for reject, rerun, resume, cancel, close, approve-*):

```bash
$R $BIN/bbp human --dir $D --task $T --dry-run yes claim $A drop
$R $BIN/bbp human --dir $D --task $T claim $A drop
$R $BIN/bbp verify --dir $D --task $T
```

If the lenses never push A to the line, that is a finding, not a failure to fix by hand (1166 section 9, last paragraph). Do not change A's stakes or push evidence by hand.

## Paste back to Claude

- `~/smoke-1-run.out` (the run record) and the tail of `~/smoke-1-run.err`
- `~/smoke-1-card-after.json` (`bbp card`)
- `~/smoke-1-verify-before.txt` and `~/smoke-1-verify-after.txt` (`bbp verify`)
- `~/smoke-1-lens-record.txt` (`lens record`)
- `~/smoke-1-why-A.json` (`bbp why --claim` for A) and `~/smoke-1-why-cand.json`
- `~/smoke-1-inbox.json`, what the board's inbox showed, and what you tapped
- the `ls -dZ` lines, any AVC denials, and the `lens check` lines

Claude writes `1166-smoke-run-record.md` from these (outcome, calls and tokens per lens, refusals by code, each lens's posts, human actions, surprises).

## What could break

- **Empty replies from the reasoning model.** `qwen3.6-35b-a3b` is a reasoning model: with a small `max_tokens` a probe returned empty `content` and only `reasoning_content`. orch's `MAX_OUTPUT_TOKENS` is 4096. Check that the lenses' replies are non-empty: a run with many `refused` or no-post rounds from `local` and `local-rebel` is this, not a lens finding.
- **Serialised calls.** llama-swap holds one large model at a time, so calls queue and swapping `gpt-oss-120b` and Qwen may dominate. Expect tens of minutes; the 90-minute box is real. `CALL_DEADLINE` in `lens/bbp.rs` bounds `bbp mcp` calls (30 s), not model calls. The `openai-http` model call has one deadline for connect, write and read together (`orch-openai/src/client.rs:124`), default `HTTP_TIMEOUT` 300 s (`rusty_orch/src/lens/registry.rs:28`), settable per provider with `timeout_secs`, 1 to 86400 (`registry.rs:271`). A ~30 s reply fits; a cold load of `gpt-oss-120b` over 300 s would time out as a transient error, so if `local-high` calls time out, set `"timeout_secs": 900` on that provider in `lens-local.json`. Cold-load time itself is unknown: check the first `local-high` call's duration in `~/smoke-1-run.err`.
- **Sandbox probe at open.** `bbp open --mode claims` refuses with `bad_claims_config` `sandbox:` if `orch-runner` cannot confine runs on this host (user namespaces or SELinux). Nothing is built, but the open still needs the probe. Check `ausearch -m avc` after a refusal.
- **Rebel is not independent.** `local-rebel` is the same Qwen weights under another id; the three candidates are two models. Noted in the record.
- **Executed-evidence refusals.** Small models may post `evidence_kind: executed`; the host refuses it (no candidate, no runner report): a wasted call counted as refused, no score change.
- **Lenses post nothing, or opinion only, or malformed JSON.** Then A never reaches 12 (criterion 4). A finding.
- **Holdout never runs here.** Only staged, hashed and validated at open. By code reading, the sandboxed run supports the suite: `TMPDIR` is set to a writable scratch directory beside the checkout (`runner.rs:486-502`), landlock grants every handled right including make-symlink, make-dir and remove on writable roots (`platform-linux/src/sys/security.rs:112-127`), and `mktemp`, `touch`, `ln`, `rm` and `grep` live under the readable `/bin` and `/usr`. Still to confirm in the first #600 builder run: that the host's coreutils are GNU (`touch -d '40 days ago'` and `touch -h -d`).
- **Card link refused.** The link needs the `Build` capability on the connector's role, and a card that already has a different execution refuses a second link; the inbox cross-check against `bbp inbox` still holds without it.
- **Wrong label.** If `ls -dZ /srv/lel/toy` still says `container_file_t`, orch-runner host processes may be denied; re-run `sudo restorecon -Rv /srv/lel/toy` only.
