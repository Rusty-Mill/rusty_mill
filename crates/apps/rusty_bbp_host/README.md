# rusty_bbp_host

The package also builds [`codex-bbp-harness` and `claude-bbp-harness`](docs/provider-launchers.md):
a turn-bound author and an MCP-only independent reviewer for Orch's two-role
example. Their contract tests are offline; the Linux real-provider smoke is opt-in.

Host commands for the Blackboard Protocol over a `rusty_bbp` file store. Stage 3 of the implementation plan: the per-turn MCP server, the human channel, the sandboxed runner and the moderator loop.

## The process is the invocation

`bbp mcp` serves exactly one turn. It reads `BBP_DIR`, `BBP_TASK`, `BBP_PRINCIPAL` and `BBP_TURN` from its environment, derives that turn's execution token, and once the turn has ended refuses every tool call except the free `task_card` and the exact replay of an operation the turn already accepted (R15: a retry after a lost response gets the original answer; a different payload under the same op or any fresh op is refused). The model never sees the token and cannot change which turn the process speaks for; a late call from a stale inference reaches a server that answers "turn has ended". That is the invocation-bound token from the design reviews, realised by process lifetime rather than in-process cancellation. The host launches one `bbp mcp` per granted turn and lets it die with the turn. With `bbp open --live-turns N` (N up to the roster size) several turns are live at once: the moderator runs one harness per live turn, kills a harness whose turn ended (a transition revokes them all), and aborts only the turn of a harness that exited.

Transport is newline-delimited JSON-RPC 2.0 on stdio: `initialize`, `ping`, `tools/list`, `tools/call`. Five tools: `task_card`, `read`, `get_artifact`, `post`, `put_artifact`. Artifact bytes are encoded by the server from the typed fields the agent sends (`rusty_bbp::codec`), so the stored payload derives from the stored bytes by construction, and the driver's boundary check confirms it.

An MCP client configuration for Claude Code, one per role, looks like:

```json
{"mcpServers": {"bbp": {"command": "bbp", "args": ["mcp"],
  "env": {"BBP_DIR": "/srv/bbp", "BBP_TASK": "T1", "BBP_PRINCIPAL": "coder", "BBP_TURN": "7"}}}}
```

## Commands

```text
bbp mcp                                   serve one turn over stdio
bbp metrics --dir D --task T              concurrency metrics of a task's log, as JSON (`bbp mod` prints `elapsed_ms`)
bbp open    --dir D --task T --repo R --brief FILE --roster FILE [--human ID] [--profiles FILE]
bbp assign  --dir D --task T --role ROLE --principal ID --vendor V
bbp card    --dir D --task T              the core card plus the task's scheduling `policy`, as JSON
bbp why     --dir D (--task T | T) [--turn N]   why each turn was granted, replayed from the log, as JSON
bbp tick    --dir D --task T              fire deadlines and grant the first turn; `bbp mod` does this itself
bbp runner  --dir D --task T --repo-path PATH --work DIR [--confine sandbox|none]
bbp mod     --dir D --task T --repo-path PATH --work DIR --agents FILE [--confine sandbox|none] [--poll-ms N] [--max-wall-secs N] [--end-grace-secs N]
bbp human   --dir D --task T [--rev N] VERB ARGS...
  approve-plan ART | approve-merge ART RUN | reject STATE REASON... | decision MSG accept|reject NOTE...
  ask ROLE TEXT... | answer MSG TEXT... | rerun ART | receipt ART REVISION | resume STATE
  extend FIELD LIMIT | cancel REASON...
```

`bbp why` answers "why did this role get this turn?" for a task of either policy. The log records who was granted a turn, not why, so it recomputes the reason: it folds the log to the event before each `turn_granted`, runs `wake_set` on that state, and applies the policy's pick rule. Read-only. Per grant it prints the turn, role, state and kind, then `how`:

- `{"WakeSet": "<reason>"}`: an event-driven task took the first selectable role of `wake_set` (`State`, `Addressed`, `Artifact`); `wake_set` lists every entry with its reason, observers included.
- `"Fallback"`: event-driven, nothing selectable, so the turn-based choice (the default holder).
- `"DefaultHolder"`: a turn-based task's grant. With one live turn it also carries `shadow`: what `wake_set` would have woken (`would`), `comparison` (`agree`, `fallback` or `disagree`), and, if `bbp shadow` kept a record for the grant, that record's `logged_rules` and whether it equals the replay (`logged_same`).
- `{"Explicit": ...}`: a forced grant that bypassed the policy: `Settlement` (a request was settled), `Consultation` (a yield or `ask`) or `ConsultationReturn`.
- `{"Unexplained": {"replayed": ...}}`: the replayed rule picks another role, so the reason is not known. This is not a match; it means the log or the build has drifted.

The reason is reconstructed, not recorded, and `wake_rules` is the version of the build that replays, which may be newer than the one that ran. The pick rule is the core's `pick_from`, the function the engine's `pick_next` calls, so `bbp why` and the engine cannot disagree about it.

`--dir` and `--task` fall back to `BBP_DIR` and `BBP_TASK`. `reject`, `rerun`, `resume` and `cancel` take `--rev N`, the card revision the human saw (`bbp card` prints it), and refuse to run without it; the core answers `stale_rev` when the card has moved on, so a delayed command is refused rather than applied to newer work.

## Status

Stages 3a to 3c of the implementation plan are in: the per-turn MCP server, the human channel, the sandboxed runner and the moderator. Execution tokens and run secrets are digests keyed by the task directory's master secret, 32 random bytes from `rusty_rand` (see "The master secret"). What is not here: an agent harness sandbox (the harness is trusted with the directory; see "The moderator"), and the live proving run, whose runbook is [`docs/proving-run.md`](docs/proving-run.md).

Tier S: `rusty_bbp`, `rusty_serde`, `rusty_rand`.

## The runner

`bbp runner` serves the task's selected run once: it clones the repository at `--repo-path` into `--work/run-N`, checks out the candidate's base, applies its diffs in order with `git apply --index`, runs every profile of the task's frozen profile set, then stores the log and the report under the run secret. Any failure before the profiles run (clone, checkout, a diff that does not apply) is an `error` report with no tree id; a profile that exits non-zero makes the run `failed`; a wall-clock timeout is `error`.

The profile set is frozen when the task opens (`--profiles FILE`, else one shell profile running `cargo test`) at `<dir>/profiles/<task>.json`, and its SHA-256 is the task's `profile_digest`. The runner re-reads and re-hashes that file before every run and refuses to run on a mismatch, so a report can only ever describe the commands the task was opened with. The file holds the profiles (program and arguments), the absolute directories the workload may read beyond the checkout, its complete environment, and its limits (CPU, wall, memory, file size, open files, processes).

Three things every run gets that the profile cannot name, because they are per run or belong to the sandbox rather than the task: the checkout is writable; a scratch directory beside it (`run-N.tmp`) is writable and is the workload's `TMPDIR` unless the profile sets one, because a linker writes its temp files there and `/tmp` is unreachable; and `/dev/null` is readable and writable, because std opens it in the child whenever a process is spawned with a null stdio (cargo starting rustc), so without it no child can be started at all. Inside the writable roots files may also be renamed and linked across directories (Landlock `REFER`), which rustc does when it moves an `.rmeta` into `deps/`. The first proving run failed on all three before any test ran; `tests/runner_e2e.rs` now exercises them.

The scratch directory is the one path the runner itself grants, so it is made fresh per execution: a symlink at `run-N.tmp` is refused (its target would become a writable root), a leftover from an earlier attempt is removed, and the directory is created exclusively with mode `0700` and checked to resolve directly under `--work`. The sandbox helper opens granted paths by name, so the one actor who could still swap the directory between that check and the grant is whoever can write `--work`. The runner therefore requires `--work` to be owned by its own user and writable by nobody else, and refuses the run otherwise; within that boundary the only such actor is the moderator's user, which is trusted with the task directory already (see "The moderator").

The workload runs under `rusty_sandbox`: Landlock restricts the filesystem to the read roots plus the checkout (the only writable root), seccomp refuses `socket(2)` outright (no network, Unix or abstract socket; anonymous socketpairs, which rustc needs to start its linker, still work), rlimits bound the rest, and the process group is killed at the wall limit. The report records the policy as `landlock+seccomp:no-endpoints`. The helper is this same binary re-invoked as `bbp __sandbox`. A sandbox the kernel cannot set up means nothing runs and the run is `error`, never a silent unconfined pass. `--confine none` runs the workload through plain `std::process` for trusted local use; the report's `sandbox` field then says `unconfined`, so a reviewer can see it.

Diffs reach `git apply --index` on standard input; the supervisor writes no file into the checkout, so a symlink the base tree or an earlier diff plants cannot redirect a trusted write outside it. If a runner stored the log and died before the report, the next runner does not execute again: the result that log described died with it, so the run is reported `error` against the log on record and the human decides whether to `rerun`.

The runner is Unix-shaped: profile sets take Unix absolute read roots and the default profile is `/bin/sh`. On Windows `bbp runner` compiles and reports every run as `error` before anything executes; its end-to-end tests are compiled on Unix only.

## The moderator

`bbp mod` is the loop that makes the core's decisions happen. Every poll it re-reads the store, fires deadlines (`Tick`), starts the runner once for a selected run with no report, launches one agent harness for a newly granted turn, and reaps a harness whose turn has ended. A harness whose turn ended gets `--end-grace-secs` (default 60) to exit on its own and print its final `result` line, which prices the turn; then it is killed. A cap or wall stop kills at once. Human gates are states with no turn, so the loop waits through them; `bbp human` from another shell moves the task on. It returns when the task is `closed` or `cancelled`, or fails after `--max-wall-secs`; at `escalated` it keeps polling until a human command moves the task on (`resume` or `cancel` are the usual choices; `reject` back to `planning` also leaves it).

One moderator per task: the loop holds an exclusive lock on `<dir>/mod/<task>.lock` for its lifetime and a second `bbp mod` on the same task is refused. A turn that is live before the loop's first tick belongs to an invocation this moderator cannot see or stop (a harness an earlier moderator launched), so it aborts that turn; the first turn of a freshly assigned task is granted by that tick and is the loop's own. On a stale turn it aborts: the old `bbp mcp` answers "turn has ended" to anything but the free card and an exact replay, and the replacement turn is launched here with a fresh token. Gates have no turn and keep waiting.

Every harness runs in its own process group and is killed, group and all, when its turn ends, when the task closes or is cancelled, at the wall limit, and on any error the loop returns, so an inference CLI's children do not outlive the moderator; a kill that fails is reported, not ignored. The runner works on its own thread, so cancellation and the wall limit are honoured while a profile is still executing (the profile itself is bounded by its sandbox wall limit).

### Admission

`--slots DIR --agent-slots N --validator-slots M` caps how many harnesses (agents) and runners (validators) run at once across every moderator, of any task, that names `DIR`. A slot is an exclusive kernel lock on `DIR/<class>-<n>.lock`; the moderator takes a free one before it launches a harness or a runner, and a granted turn or selected run with no free slot waits for a later poll (or its deadline). The locked file becomes the harness's stdin or the runner's stdout, so the lock lives in an open file description the whole process tree shares. A launcher may replace a harness's stdin, so the moderator also passes the lock to a `bbp slot-hold --group G` process in its own process group, which keeps it until the harness's process group has no member left. The slot is released only when the moderator has reaped its child, or, if the moderator dies first, when the child and everything it started are gone. Nothing is recorded that a crash could leave behind, and a restarted moderator gets the slot back exactly when its holder has exited. The first moderator records the capacity in `DIR/capacity`; one configured differently is refused. Like the runner, these guarantees are Unix-shaped and proven on Linux; on Windows the slots lock and count, but a crash is not claimed to keep them. `tests/admission_e2e.rs` races four moderators through two agent slots and one validator slot, and kills a moderator while its harness holds the only slot, once with a harness that keeps its stdin and once with one that replaces it.

The harness is trusted with the directory. It runs as the moderator's user, so a harness with shell or file tools can read `<dir>/master` or call `bbp human`; `bbp mcp` is the fence for the model's tool calls, not for the harness process. Run a harness you do not trust under its own sandbox or user, with the task directory unreadable to it except through the MCP server.

`--roster FILE` (on `bbp open`) is a JSON array of roles: id, `turn_order` and the capability fields of the spec's Role table. The roles are the orchestrator's to define; the host passes them on and the core validates and records them. `docs/proving/roster.json` is the four-role example of spec Appendix C.1. An invalid roster is refused with `BadRoster` and no task exists; a roster cannot change afterwards.

`--agents FILE` names one launcher per role id, keyed by the ids of the task's roster. Launch policy for a role lives here, not in the host code: `builtin_tools` says whether that role's harness keeps its shell and file tools (default `false`). The proving config gives it to the `coder` only:

```json
{"planner": {"program": "claude", "args": ["-p", "You are the planner. Use the bbp tools.", "--mcp-config", "{mcp_config}"]},
 "coder":   {"program": "claude", "args": ["-p", "You are the coder.",   "--mcp-config", "{mcp_config}"], "builtin_tools": true},
 "tester":  {"program": "claude", "args": ["-p", "You are the tester.",  "--mcp-config", "{mcp_config}"]},
 "reviewer":{"program": "claude", "args": ["-p", "You are the reviewer.","--mcp-config", "{mcp_config}"]}}
```

Arguments may use `{dir}`, `{task}`, `{principal}`, `{turn}`, `{role}`, `{bbp}` (this binary) and `{mcp_config}`: a Claude Code MCP config written whole-or-nothing per turn at `<dir>/mcp/<sha256(task)>/turn-N.json` that starts `bbp mcp` bound to that turn. The harness also gets `BBP_DIR`, `BBP_TASK`, `BBP_PRINCIPAL`, `BBP_TURN`, `BBP_ROLE`, `BBP_BIN` and `BBP_MCP_CONFIG` in its environment; its output goes to `<dir>/agents/<sha256(task)>/turn-N.log`. A harness that exits without ending its turn forfeits it: the moderator aborts the turn so the protocol moves on. A role without a launcher is left to its deadline.

### Live smoke

Run once by hand with Claude Code 2.1.295 as the planner (`claude -p … --mcp-config {mcp_config} --strict-mcp-config --allowedTools mcp__bbp__…`, no coder, tester or reviewer launcher): the moderator granted turn 1 and launched the harness; the model read the card and the brief, stored a spec with two acceptance criteria (artifact 2, 392 bytes) and posted a gated `request_decision` referencing it; the core moved the task to `plan_gate` and ended the turn; the moderator stopped the harness and waited at the gate until its wall limit. The scripted agents in `tests/moderator_e2e.rs` cover the rest of the path in CI.

## The master secret

`bbp open` creates `<dir>/master`, 32 random bytes from `rusty_rand`, owner-readable only, published with a no-clobber link so two first opens racing each other both come back with the key on disk and no later process overwrites a key an earlier one already uses. Every execution token and run secret is a digest keyed by it, and neither ever enters the log. A process that can read the log but not the master cannot forge a token; the agent harness sees neither, only the per-turn `bbp mcp` server, which derives the token for its own turn.

### Structured task-host reads and retryable human actions

`bbp show --dir D --task T` returns a public `rusty_bbp::TaskView`, or JSON null
when the task has no log. It includes the card, immutable roster, assigned
principals, proposed spec, pending request body, merge receipt, cancellation
reason and escalation. It does not expose tokens, the master key or run secrets.
`bbp card` remains unchanged.

`bbp human --dir D --task T --fence REV --op OP [--rev REV] VERB ...` applies any
human verb only to the displayed revision inside the driver's transaction.
Successful retries with the same OP and payload replay before fence/terminal
checks. Destructive verbs still require `--rev`. Existing unfenced direct CLI
usage remains available; Orch always supplies the extra fence and stable op.
Approval still does not close a task: merge externally, then record the receipt.

Malformed fenced human input returns `Rejected { code: BadHumanInput, ... }`
without dispatching a command. This allows task hosts to show a correctable new
question. I/O errors remain process failures; they are not human-input refusals.

The moderator runs `bbp runner --supervised yes` with a private stdin ownership
pipe, in a separate process group. EOF cancels its workload using the existing
sandbox job handle and waits for reaping. The worker retains the task's runner
lock through cleanup/reporting, including after moderator death. Thus a new
moderator may retry a pending run only after the old worker releases that lock.
Normal wall-limit, cancellation, and error exits wait for the worker to finish.
The standalone unsupervised runner CLI remains available for direct trusted use.

Runner cancellation is registered before sandbox setup polling, so owner-pipe
EOF also kills a helper that has not exec'd its workload yet. Startup errors
retain executor ownership through group containment and reaping; only then may
the runner report an error and release its task lock. A delayed-helper test
covers EOF during setup, cancellation latched before registration, lock
retention and a forbidden later workload launch.

Provider launch-loop protection is replayed from the existing event log: three
no-progress forfeits/timeouts for one role pause automatic launches across
restarts. `bbp mod-health` reports the fault; after repairing it, the owner resets
with `--reset yes --rev DISPLAYED_REV` under the moderator lock. Orch retains its
hosted task and call ledger on this diagnostic. See
[provider launchers](docs/provider-launchers.md) for recovery and the generated
`bbp profiles rust-lld --sysroot PATH` profile, which bounds linker threads before
task creation without increasing memory limits or changing frozen profiles.
