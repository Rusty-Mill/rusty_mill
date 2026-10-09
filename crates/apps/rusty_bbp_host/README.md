# rusty_bbp_host

Host commands for the Blackboard Protocol over a `rusty_bbp` file store. Stage 3 of the implementation plan: the per-turn MCP server, the human channel, the sandboxed runner and the moderator loop.

## The process is the invocation

`bbp mcp` serves exactly one turn. It reads `BBP_DIR`, `BBP_TASK`, `BBP_PRINCIPAL` and `BBP_TURN` from its environment, derives that turn's execution token, and once the turn has ended refuses every tool call except the free `task_card`. The model never sees the token and cannot change which turn the process speaks for; a late call from a stale inference reaches a server that answers "turn has ended". That is the invocation-bound token from the design reviews, realised by process lifetime rather than in-process cancellation. The host launches one `bbp mcp` per granted turn and lets it die with the turn.

Transport is newline-delimited JSON-RPC 2.0 on stdio: `initialize`, `ping`, `tools/list`, `tools/call`. Five tools: `task_card`, `read`, `get_artifact`, `post`, `put_artifact`. Artifact bytes are encoded by the server from the typed fields the agent sends (`rusty_bbp::codec`), so the stored payload derives from the stored bytes by construction, and the driver's boundary check confirms it.

An MCP client configuration for Claude Code, one per role, looks like:

```json
{"mcpServers": {"bbp": {"command": "bbp", "args": ["mcp"],
  "env": {"BBP_DIR": "/srv/bbp", "BBP_TASK": "T1", "BBP_PRINCIPAL": "coder", "BBP_TURN": "7"}}}}
```

## Commands

```text
bbp mcp                                   serve one turn over stdio
bbp open    --dir D --task T --repo R --brief FILE [--human ID] [--profiles FILE]
bbp assign  --dir D --task T --role ROLE --principal ID --vendor V
bbp card    --dir D --task T
bbp tick    --dir D --task T
bbp runner  --dir D --task T --repo-path PATH --work DIR [--confine sandbox|none]
bbp mod     --dir D --task T --repo-path PATH --work DIR --agents FILE [--confine sandbox|none] [--poll-ms N] [--max-wall-secs N]
bbp human   --dir D --task T VERB ARGS...
  approve-plan ART | approve-merge ART RUN | reject STATE REASON... | decision MSG accept|reject NOTE...
  ask ROLE TEXT... | answer MSG TEXT... | rerun ART | receipt ART REVISION | resume STATE
  extend FIELD LIMIT | cancel REASON...
```

`--dir` and `--task` fall back to `BBP_DIR` and `BBP_TASK`. Human actions that change state carry the card revision the human saw, so a stale command is refused rather than applied to newer work.

## Not here yet

Stage 3b adds `bbp runner`, the sandboxed test supervisor on `rusty_sandbox`. Stage 3c adds `bbp mod`, the loop that grants turns, launches one agent harness per turn with this server, and starts runs. Tokens and run secrets are still the deterministic derivations from stage 1; `rusty_rand` replaces them when the moderator exists to hand them out.

Tier S: `rusty_bbp`, `rusty_serde`, `rusty_rand`.

## The runner

`bbp runner` serves the task's selected run once: it clones the repository at `--repo-path` into `--work/run-N`, checks out the candidate's base, applies its diffs in order with `git apply --index`, runs every profile of the task's frozen profile set, then stores the log and the report under the run secret. Any failure before the profiles run (clone, checkout, a diff that does not apply) is an `error` report with no tree id; a profile that exits non-zero makes the run `failed`; a wall-clock timeout is `error`.

The profile set is frozen when the task opens (`--profiles FILE`, else one shell profile running `cargo test`) at `<dir>/profiles/<task>.json`, and its SHA-256 is the task's `profile_digest`. The runner re-reads and re-hashes that file before every run and refuses to run on a mismatch, so a report can only ever describe the commands the task was opened with. The file holds the profiles (program and arguments), the absolute directories the workload may read beyond the checkout, its complete environment, and its limits (CPU, wall, memory, file size, open files, processes).

The workload runs under `rusty_sandbox`: Landlock restricts the filesystem to the read roots plus the checkout (the only writable root), seccomp forbids sockets, rlimits bound the rest, and the process group is killed at the wall limit. The helper is this same binary re-invoked as `bbp __sandbox`. A sandbox the kernel cannot set up means nothing runs and the run is `error`, never a silent unconfined pass. `--confine none` runs the workload through plain `std::process` for trusted local use; the report's `sandbox` field then says `unconfined`, so a reviewer can see it.

The runner is Unix-shaped: profile sets take Unix absolute read roots and the default profile is `/bin/sh`. On Windows `bbp runner` compiles and reports every run as `error` before anything executes; its end-to-end tests are compiled on Unix only.

## The moderator

`bbp mod` is the loop that makes the core's decisions happen. Every poll it re-reads the store, fires deadlines (`Tick`), starts the runner once for a selected run with no report, launches one agent harness for a newly granted turn, and reaps a harness whose turn has ended. Human gates are states with no turn, so the loop waits through them; `bbp human` from another shell moves the task on. It returns when the task is `closed` or `cancelled`, or fails after `--max-wall-secs`.

`--agents FILE` names one launcher per role:

```json
{"planner": {"program": "claude", "args": ["-p", "You are the planner. Use the bbp tools.", "--mcp-config", "{mcp_config}"]},
 "coder":   {"program": "claude", "args": ["-p", "You are the coder.",   "--mcp-config", "{mcp_config}"]},
 "tester":  {"program": "claude", "args": ["-p", "You are the tester.",  "--mcp-config", "{mcp_config}"]},
 "reviewer":{"program": "claude", "args": ["-p", "You are the reviewer.","--mcp-config", "{mcp_config}"]}}
```

Arguments may use `{dir}`, `{task}`, `{principal}`, `{turn}`, `{role}`, `{bbp}` (this binary) and `{mcp_config}`: a Claude Code MCP config written per turn at `<dir>/mcp/turn-N.json` that starts `bbp mcp` bound to that turn. The harness also gets `BBP_DIR`, `BBP_TASK`, `BBP_PRINCIPAL`, `BBP_TURN`, `BBP_ROLE`, `BBP_BIN` and `BBP_MCP_CONFIG` in its environment; its output goes to `<dir>/agents/turn-N.log`. A harness that exits without ending its turn forfeits it: the moderator aborts the turn so the protocol moves on. A role without a launcher is left to its deadline.

## The master secret

`bbp open` creates `<dir>/master`, 32 random bytes from `rusty_rand`, owner-readable only. Every execution token and run secret is a digest keyed by it, and neither ever enters the log. A process that can read the log but not the master cannot forge a token; the agent harness sees neither, only the per-turn `bbp mcp` server, which derives the token for its own turn.
