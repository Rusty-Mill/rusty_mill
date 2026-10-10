# Proving run: first live task

The first task driven end to end by live models under `bbp mod`: four Claude Code harnesses as Planner, Coder, Tester and Reviewer, one human at the gates, one sandboxed runner. Its purpose is a record, not a pass: where the rules bit, what the budgets hit, where a role stalled. That record answers the spec's open questions Q1 to Q3 (`docs/spec/bbp-v0.4-implementation-spec.md`, "Open questions for after the proving run"). Never raise a limit to make the task pass.

Files in `docs/proving/`: `brief.md` (the task), `profiles.json` (the frozen test profile), `agents.json` (one launcher per role). Paths in the last two assume `/tmp/bbp` and a user `nano`; edit before use.

Linux only: the runner is Unix-shaped and reports every run as `error` on Windows (README, "The runner"). On a Windows machine run everything below inside WSL2, build `bbp` there, and keep the store outside `/tmp` if WSL may restart (`/tmp` is wiped).

## 1. Build

```sh
cargo build --release -p rusty_bbp_host
export MILL="$PWD" P="$PWD/crates/apps/rusty_bbp_host/docs/proving"
export PATH="$MILL/target/release:$PATH"   # `bbp` on PATH: agents.json launches it by name through {mcp_config}
```

`MILL` and `P` are absolute so the later steps work from any directory; step 2 leaves the shell in `/tmp/bbp/target`.

Before anything else, prove the harness can reach the model: `claude -p "say ok" --output-format json | jq .is_error` must print `false`. `claude auth status` can say `loggedIn: true` with an expired token; the moderator would then forfeit every turn after about two minutes of 401 retries.

## 2. Target repository

A throwaway crate with no dependencies, so `cargo test --offline` works with the network blocked.

```sh
mkdir -p /tmp/bbp && cd /tmp/bbp
cargo new --lib target && cd target
mkdir tests && cat > tests/slug.rs <<'T'
use target::slug;
#[test] fn lowercases_and_joins() { assert_eq!(slug("Hello, World!"), "hello-world"); }
#[test] fn collapses_runs()      { assert_eq!(slug("a  --  b"), "a-b"); }
#[test] fn trims_edges()         { assert_eq!(slug("--x--"), "x"); }
#[test] fn empty()               { assert_eq!(slug(""), ""); }
T
cargo build -q && cargo test -q 2>&1 | grep -q 'unresolved import' && git add -A && git commit -qm "slug: failing tests"
cargo fetch   # warms ~/.cargo so the sandboxed `cargo test --offline` needs nothing from the network
```

## 3. Open the task

```sh
export BBP_DIR=/tmp/bbp/store BBP_TASK=slug-1
bbp open   --repo /tmp/bbp/target --brief $P/brief.md --human nano --profiles $P/profiles.json
bbp assign --role planner  --principal planner  --vendor anthropic
bbp assign --role coder    --principal coder    --vendor anthropic
bbp assign --role tester   --principal tester   --vendor anthropic
bbp assign --role reviewer --principal reviewer --vendor anthropic
```

`profiles.json` must name the toolchain the workload needs: `read_roots` with `~/.cargo` and `~/.rustup`, `PATH` with `~/.cargo/bin`. The sandbox allows nothing else, and the set is frozen at open: a change afterwards means a new task.

## 4. Run

Shell A, the moderator (returns at `closed` or `cancelled`; at `escalated` it keeps running until a human command moves the task on, usually `bbp human resume` or `cancel`, so decide, do not just wait):

```sh
bbp mod --repo-path /tmp/bbp/target --work /tmp/bbp/work --agents $P/agents.json --max-wall-secs 7200
```

Shell B, the human (a fresh shell: set the same variables first):

```sh
export MILL=/path/to/rusty_mill            # its own command: the next line expands $MILL before export runs
export BBP_DIR=/tmp/bbp/store BBP_TASK=slug-1 PATH="$MILL/target/release:$PATH"
watch -n 5 'bbp card | python3 -m json.tool | head -40'
T=$BBP_DIR/tasks/$(printf %s "$BBP_TASK" | sha256sum | cut -c1-64).log   # the task's event log, one JSON array per line
# at plan_gate: the proposed spec is refs[0] of the pending gate request, not card.spec (that is the approved spec, null until now)
SPEC=$(jq -r --argjson m "$(bbp card | jq .pending_request)" '.[] | .MessageAppended? // empty | select(.id==$m) | .draft.refs[0].Art.id' "$T")
bbp human approve-plan  "$SPEC"
bbp human approve-merge ART RUN             # at merge_gate: the current candidate and its selected run, both on the card
git -C /tmp/bbp/work/run-RUN diff --cached > /tmp/bbp/cand.diff   # the runner's checkout holds the applied candidate
git -C /tmp/bbp/target apply --index /tmp/bbp/cand.diff && git -C /tmp/bbp/target commit -qm "slug (BBP slug-1)"
bbp human receipt ART "$(git -C /tmp/bbp/target rev-parse HEAD)"   # closes the task
```

States in `bbp human` are lower snake case (`planning`, `plan_gate`, `build`, `test`, `review`, `merge_gate`, `approved`, `escalated`); the card prints them capitalised.

Destructive verbs (`reject`, `rerun`, `resume`, `cancel`) take `--rev N`, the card revision you are looking at. A stale revision is refused; re-read the card and decide again.

### Recovering from an environment fault

A non-gate `request_decision` from the Coder or Tester (the launcher prompts tell them to post one when a run fails for a reason no code change can fix) shows on the card as `pending_request` with no turn, and nothing is granted while it is open. Settling it regrants the requester; it does not refresh anything else, so the order matters:

1. Fix the environment outside the store (install the toolchain file, fix a permission, point a symlink at a readable target). If the fix needs a change to the frozen profile, read roots, environment or limits, stop: the digest is frozen at open, so `cancel --rev N` this task and open a new one with the corrected `profiles.json`.
2. If the requester is the Tester (state `test`, the failed run is still the selected one): `bbp human rerun CAND --rev N` with the candidate id from the card. The core revokes the failed run and selects a fresh one; the moderator runs it; wait until the card's `run` shows a terminal status. A stale `--rev` is refused; re-read the card and repeat. Do not answer the request first: a Tester regranted before the rerun reads the same failed report and asks again.
3. Read the fresh report before settling anything:
   - `passed`, or `failed` with the tests actually executed (the log shows test output, not a toolchain error): settle. `bbp human answer MSG fixed, rerun passed` or `answer MSG environment fixed; the run failed on the tests`, where `MSG` is the card's `pending_request`. The requester is regranted; the Tester judges that report, a Coder (state `build`, no run to redo) reapplies its diff in a fresh clone and resubmits.
   - `failed` with the same environment error: the fix did not take. Do not settle; nothing is granted while the request is open. Fix again and rerun again (step 2).
   - `error` (the runner could not execute the profile, or it hit the wall limit): the task is now `escalated` and the request is still pending. Settling it does not bring the Tester back. Fix the environment, then `bbp human resume test --rev N`, which starts a fresh run; when its report is in, settle as above.
   - To say the fault is not environmental: `bbp human decision MSG reject NOTE`. The requester is regranted and judges the run as it stands; it may open a new request, since the old one is settled.

An `answer` settles only the request it replies to, and a `decision` only the request it names; any other human message leaves the request pending. No iteration is spent on any of this. The first run's roles did not have the route in their prompts and spent the whole iteration budget on revise/resubmit instead. `tests/runner_e2e.rs` has each branch end to end: `an_environment_fault_is_recovered_by_rerun_then_settlement` (the straight path), `a_failed_rerun_keeps_the_request_open_until_a_run_passes`, `an_error_rerun_escalates_and_resume_then_settlement_recovers`, `a_rejected_request_regrants_the_tester_who_may_ask_again`, and `a_coder_request_is_settled_by_an_answer_and_the_coder_resubmits`.

Logs: `$BBP_DIR/agents/<sha256(task)>/turn-N.log` per harness, `$BBP_DIR/mcp/<sha256(task)>/turn-N.json` the config each saw, `/tmp/bbp/work/run-N` the runner's checkouts.

### What the harness can do, and what the log shows

`agents.json` restricts each harness two ways. `--tools ""` removes the built-in tools from the Planner, Tester and Reviewer, so their only actions are the five bbp tools (`--allowedTools` alone would pre-approve those calls without removing Read, Bash and the rest). One exception on Claude Code 2.1.296: `system/init` still lists `LSP` under `--tools ""`; the first run saw it listed on every MCP-only turn and never used. Treat `LSP` in the list as expected and any other built-in tool as a launcher defect. The Coder keeps `Bash,Edit,Read,Write` for its own clone. This is the model's tool surface, not an OS sandbox: the process still runs as your user, which is the harness-isolation question the record is meant to answer.

Every launcher runs with `--output-format stream-json --verbose`, so `turn-N.log` is one JSON object per line: `system/init` (the tools and MCP servers the model saw), `assistant` messages with their `tool_use` blocks, `user` messages with the `tool_result` each call returned (a bbp refusal is a result with `is_error`), and a final `result`. Each line is written as it happens.

What the log does and does not promise. A `tool_use` is written before the call is made, so every call the model issued is on disk. The call that ends the turn (a candidate, a verdict, a gated request) is different: the core records the turn's end before `bbp mcp` renders the response, and the moderator may see that and kill the process group first. So the terminal call's `tool_result`, the `result` line and anything after them may be missing, by design, and the log alone cannot say whether that call was accepted. The store can: the card and its artifacts are the record of what the core admitted. Read the harness's intent from the log and its effect from the card.

Validate this once, after turn 1 ends, before trusting the run:

```sh
L=$BBP_DIR/agents/$(printf %s "$BBP_TASK" | sha256sum | cut -c1-64)/turn-1.log
jq -r 'select(.type=="system" and .subtype=="init") | .tools[]' "$L"            # an MCP-only role lists only mcp__bbp__* (plus LSP, see above)
jq -c 'select(.type=="assistant") | .message.content[] | select(.type=="tool_use") | {name, input}' "$L"   # every call, in order
jq -r 'select(.type=="user") | .message.content[] | select(.type=="tool_result" and .is_error==true) | .content' "$L"   # refusals
bbp card
```

The last `tool_use` must be the call the card says ended the turn. For the Planner: `mcp__bbp__post` with `input.kind == "request_decision"`, `input.gate == true` and `input.refs == ["art:<SPEC>"]`, where `SPEC` is derived from the task log as in shell B (the message whose id is the card's `pending_request`, `refs[0]`); the card is in `plan_gate` with `pending_request` set and `spec` still `null`, because `spec` is the approved spec and nothing is approved yet. Match the terminal call by name and input, not by "a post or a put happened": the Planner's earlier spec `put_artifact` is also in the log and proves nothing about the gate. If the last `tool_use` is not that call, or the card did not move, the harness ended without its terminal call (it forfeited the turn, or was killed early) and the run is not evidence yet; read the log's tail and the moderator's stderr before continuing.

## 5. Record

Write `docs/proving/<date>-slug-1.md` with, in this order:

1. Outcome: `closed`, `cancelled` or `escalated`, wall time, number of turns, iterations, candidates, runs.
2. The card's final `budget` block next to the limits, and `rejections` by code. For every limit hit: which role, on which turn, doing what. This is Q1's input.
3. Each role's turns: did it finish its job in one turn, what did it read first, what did it get refused, where did it stall or loop. Quote the `tool_use` and `tool_result` lines from `turn-N.log`, not a paraphrase.
4. Human interventions: every `bbp human` command with its reason.
5. The Reviewer's verdict on each candidate, next to the Tester's. Agreement or not is Q3's first data point.
6. Anything the Coder's harness did that the protocol did not see: every `Bash` call in its logs, any write outside its clone. The other three roles have no built-in tools, so for them the question is whether `system/init` ever listed one. This decides whether harness isolation comes next.

Out of scope for this run: the 16-task comparison against the relay loop (spec, "Measurement"). That needs a published task set and rubric, and comes after the first record exists.
