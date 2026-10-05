# rusty_rsi

An AIDE²-style recursive self-improvement harness. An inner agent improves a
task's solution against a public score under a fixed budget. An outer agent
rewrites the inner agent's code and keeps a rewrite only if its grade (the
mean private, held-out score across a task suite) beats the incumbent's by
more than the measured noise.

Design, reuse decisions and scope: [ADR-0005](../../../docs/adr/0005-rsi-harness.md).

## Status

| Phase | Deliverable | State |
|---|---|---|
| P1 | `rsi-core`: domain types, accept gate, budget meter, search helpers, lineage hash chain | merged ([#472](https://github.com/Rusty-Mill/rusty_mill/pull/472)) |
| P2 | Task format, three toy tasks, sandboxed executor, out-of-process grader | merged ([#476](https://github.com/Rusty-Mill/rusty_mill/pull/476)) |
| P3 | Model broker, inner harness `a0` (AIDE0 behaviour), model clients, `rsi inner` | merged ([#478](https://github.com/Rusty-Mill/rusty_mill/pull/478)) |
| P4 | Outer loop, `rsi calibrate`, 10-step end-to-end run, `rsi report` | [#483](https://github.com/Rusty-Mill/rusty_mill/pull/483) |

## Crates

| Crate | Purpose | ADR-0002 tier |
|---|---|---|
| [`rsi-core`](crates/rsi-core) | Pure domain and ports: no I/O, no async, no clock | S (workspace foundation crates only) |
| [`rsi-runtime`](crates/rsi-runtime) | Adapters: sandboxed executor, task directories, metrics, graders, the broker and agent runner, model clients, the outer loop, lineage store, git worktrees, proposers and reports; the [toy task suite](crates/rsi-runtime/tasks) | S (workspace crates only) |
| [`rsi-cli`](crates/rsi-cli) | The `rsi` binary: `rsi calibrate`, `rsi run`, `rsi report` and `rsi inner`, plus the internal `__sandbox` and `__grade` entry points | S (workspace crates only) |
| [`rsi-harness`](harness) | a0, the inner agent; std-only, compiled by the runtime with plain `rustc`. The only code the outer loop may rewrite | S (no dependencies) |

## Invariants

- **Budget (2):** `CostMeter` refuses any operation once tokens, wall-clock
  or GPU time reach the limit; usage past the limit is still recorded. End
  to end, the broker refuses every model call and evaluation once the
  budget is spent (only `submit` stays open), and an agent that keeps
  running is killed at the wall-clock budget plus a grace period.
- **Accept gate (3):** `screen` rejects a candidate that does not beat the
  incumbent. `confirm` accepts only on a fresh-seed grade that beats it by
  more than the calibrated margin, and refuses reused seeds. End to end, a
  10-step run accepts only the candidate that clears the margin on a
  fresh, disjoint seed set; equal and within-noise candidates are rejected.
- **Lineage (4):** entries carry results rather than grades, so the grade is
  always recomputed, and a repeated `(task, seed)` result is an error.
  `LineageEntry::new` replays the gate on that evidence and refuses a
  decision that does not follow from it. `verify_chain` detects edits,
  deletions and reordering. An inner run replays from its broker
  transcript, without the model, to the same submission. `rsi report
  --replay` re-grades every stored submission bit for bit and replays
  every inner run; an altered blob or lineage line is caught.
- **Path allowlist:** a proposal may change only regular files under
  `harness/src/`. Anything else (a manifest, task data, a symlink) is
  recorded as a path violation and never built.

- **Private isolation (1):** solutions run in a fresh directory with
  read access to system directories only (Landlock). A solution that
  tries to read a task's private labels gets `PermissionError`, whether
  during public scoring or private grading. Private labels are read only by
  the separate `rsi __grade` process, and the executor fails closed when
  the sandbox cannot be enforced. The agent itself reads only system
  directories, its binary and its work directory, and cannot open any
  socket: its one channel is the broker socket it inherits as stdin.
- **Sandbox limits (5):** no internet sockets for solutions (seccomp;
  the agent gets no new sockets at all), no writes outside the work
  directory, and CPU, memory, file size and descriptor rlimits. A
  wall-clock kill takes down the whole process group.

The sandbox needs Linux with Landlock, the toy tasks need `python3`, and
building the agent needs `rustc` (`RSI_RUSTC` overrides which one).

## One inner run

```sh
cargo build -p rsi-cli
RSI_INNER_MODEL=qwen2.5-coder:7b ./target/debug/rsi inner \
  --harness crates/apps/rusty_rsi/harness \
  --task crates/apps/rusty_rsi/crates/rsi-runtime/tasks/tsp-heuristic \
  --tokens 50000 --wall-secs 600 --transcript /tmp/run.bin
```

The model is any OpenAI-compatible endpoint over plain HTTP:
`RSI_INNER_BASE_URL` (default `http://127.0.0.1:11434/v1`, a local Ollama)
and, if needed, `RSI_INNER_API_KEY`. Configuration comes from the
environment only, and the key is never logged.

## A run of the outer loop

```sh
cargo build -p rsi-cli
export RSI_INNER_MODEL=qwen2.5-coder:7b RSI_OUTER_MODEL=qwen2.5-coder:32b
TASKS=crates/apps/rusty_rsi/crates/rsi-runtime/tasks
./target/debug/rsi calibrate --tasks $TASKS --tokens 50000 --wall-secs 600 --out calibration.json
./target/debug/rsi run --tasks $TASKS --run-dir runs/first --steps 10 \
  --tokens 50000 --wall-secs 600 --calibration calibration.json
./target/debug/rsi report --run-dir runs/first --replay --tasks $TASKS
```

`--repo` defaults to the current repository. Candidates are committed under
`refs/rsi/<run>/<step>`, never on a branch; the run directory holds
`run.json`, the hash-chained `lineage.jsonl` and content-addressed
`blobs/`. Each role reads its own `RSI_<ROLE>_MODEL`, `_BASE_URL` and
`_API_KEY` (`INNER` for the agent, `OUTER` for the proposer). Running the
outer loop also needs `git` 2.25 or later.

### Codex or Claude Code as the proposer

```sh
codex login                                  # once; stored in CODEX_HOME
export RSI_OUTER_PROPOSER=codex
export RSI_OUTER_CODEX=/path/to/vendor/x86_64-unknown-linux-musl/bin/codex  # the native binary
export RSI_OUTER_MODEL=gpt-5-codex           # optional: Codex's default otherwise
```

Codex runs inside `rsi`'s sandbox, not its own: it edits a copy of the
harness (no `.git`), may write only that copy, `CODEX_HOME` and a private
temp dir, may use the network, and cannot reach the tasks or the run
directory. `RSI_OUTER_CODEX` defaults to `codex` on `PATH`, which must be
the native binary, not the npm launcher script. Proxy and certificate
variables (`HTTPS_PROXY`, `SSL_CERT_FILE`, ...) pass through.

Claude Code works the same way with its subscription login (no Anthropic
key), limited to the file tools (no commands):

```sh
claude                                       # once: /login; stored in CLAUDE_CONFIG_DIR (~/.claude)
export RSI_OUTER_PROPOSER=claude
export RSI_OUTER_CLAUDE=/path/to/claude      # the native binary; default `claude` on PATH
export RSI_OUTER_MODEL=opus                  # optional
```

### Codex as the inner model

```sh
export RSI_INNER_PROVIDER=codex              # default: openai (RSI_INNER_MODEL, _BASE_URL)
export RSI_INNER_CODEX=/path/to/codex        # optional; default `codex` on PATH
export RSI_INNER_MODEL=gpt-5-codex-mini      # optional
```

a0 still runs every call through the metered broker; each call is one
sandboxed `codex exec --json`, charged from its reported token usage (a
call that reports none is an error). Each call starts a Codex process,
so runs are slower than with a local endpoint.

Real-agent smoke tests, once logged in:
`cargo test -p rsi-cli --test agents -- --ignored` (set `RSI_OUTER_CODEX`
and `RSI_OUTER_CLAUDE`).

## Test

```sh
cargo test -p rsi-core -p rsi-runtime -p rsi-cli -p rsi-harness
```
