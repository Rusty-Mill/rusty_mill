# ADR-0005: `rusty_rsi`, an AIDE²-style recursive self-improvement harness

Status: Accepted
Date: 2026-09-30 (proposed) / 2026-10-04 (accepted)

Accepted by the owner on 2026-10-04 with all four open questions resolved
as recommended:
1. Toy-task solutions run on `python3`, standard library only.
2. This ADR stays in the root series (see Remit).
3. Hoisting a shared git-CLI crate into `libs/` (from `rsi-runtime`'s
   adapter and `sessionmgr-git`) is a separate follow-up, not part of
   P1–P4.
4. If a CI runner lacks Landlock, the fix is a CI-configuration change
   raised for sign-off; sandbox tests are never skipped.

## Remit

This ADR belongs to the root series, not a future
`crates/apps/rusty_rsi/docs/adr/`, for two reasons (ADR-0001 remit). It
records reuse decisions against five other families under ADR-0003's
layer rule. It also makes one workspace-level precedent: a workspace
member whose source is rewritten by a program, under a path allowlist.
Decisions internal to the family after this one go in the family's own
`docs/adr/`.

## Context

Weco AI's AIDE² (blog post "First evidence of recursive
self-improvement"; report arXiv:2609.26457, Alg. 1 and §2–3.5) is a
bi-level loop:

- **Inner loop.** An agent `a` edits a task's codebase against a
  *public* score `r_pub` until a fixed per-task budget `b_t` (agent
  tokens plus solution execution) runs out. It then returns one
  solution `x̂_t` of its own choosing.
- **Grade.** `g(a) = (1/T) Σ_t r_priv(x̂_t)`: the mean *private*,
  held-out score over a heterogeneous suite (ML engineering, heuristic
  algorithms, harness engineering), each task run several times.
- **Outer loop.** An agent reads prior agents and their grades,
  proposes a rewrite of the inner agent's code, grades it, and keeps it
  only if it beats the incumbent. In the published run, 7 of 99
  proposals were accepted.

The report leaves out several things we need. It does not quantify
AIDE0's budgets, seed counts or hyperparameters. It meters cost in
dollars. Its acceptance rule is a bare argmax, although it reports
run-to-run noise of about 0.02–0.045, the same size as the accepted
gains. The AIDE0 search policy is taken from `WecoAI/aideml`
(`aide/agent.py::search_policy`; `num_drafts=5`, `debug_prob=0.5`,
`max_debug_depth=3`; MIT licence, © 2024 Weco AI Ltd). We port the
**behaviour**, not code: no aideml source is copied, so no licence
notice is owed. This ADR records the consultation anyway.

Three deliberate departures from the report:

1. **Cost is tokens plus wall-clock, not dollars.** A local Ollama inner
   model costs about $0, so a dollar budget stops constraining anything.
   GPU-seconds are recorded when available, and are `None` otherwise.
2. **The accept gate is noise-aware.** A candidate that wins is
   re-graded on fresh seeds and must win by a calibrated margin (see
   Decision §5).
3. **The mutable surface is Rust, and it is sandboxed.** AIDE² edits
   Python; here the harness is one std-only Rust crate compiled per
   candidate.

## Decision

### 1. Placement and crates

A new family at `crates/apps/rusty_rsi/`, with `layer = "apps"`:

| Member | Path | Role | Deps (workspace → external) |
|---|---|---|---|
| `rsi-core` | `crates/rsi-core` | Pure domain: no I/O, no async, no clock. | `rusty_err`, `rusty_rsa` (sha256 for the lineage hash chain) → none |
| `rsi-runtime` | `crates/rsi-runtime` | Adapters for every port. | `rsi-core`, `rusty_err`, `rusty_json` (no default features), `rusty_http` (sync adapter only); on Linux `platform`, `platform-linux` (Landlock/seccomp), `rusty_libc` (rlimits, `killpg`) → none |
| `rsi-cli` | `crates/rsi-cli` | Composition root: `rsi run \| calibrate \| report`, plus hidden `rsi __sandbox` / `rsi __grade` entry points. | the two above |
| `rsi-harness` | `harness/` | **The only mutable surface**: the inner agent. It has zero dependencies (std only). A library to Cargo; the runtime compiles `src/lib.rs` as the binary (§3). | none |

There are no other crates, and none will be added before a second real
call site exists. Tasks are data, not crates:
`crates/apps/rusty_rsi/crates/rsi-runtime/tasks/<name>/{task.json, public/,
private/}`. Two deviations from this ADR's first draft, both made in P2:

- **`task.json`, not `task.toml`.** The workspace has no first-party TOML
  parser (only the external `toml` crate), and `rusty_json` is already a
  dependency, so a JSON manifest keeps `rsi-runtime` free of external
  dependencies.
- **Tasks live inside `rsi-runtime`'s directory.** CI's
  `affected_crates.py` maps a changed file to the crate whose directory
  contains it. Data outside every crate directory would never re-run the
  tests that consume it, so a task edit has to belong to a crate.

This family is `apps` and therefore may depend only on
`foundation`/`platform`/`libs`. `check_workspace_layers.py:98-101`
rejects any cross-family `apps → apps` edge. That rule decides several
of the reuse rows in Appendix A before taste does.

### 2. Ports (`rsi-core`)

The ports are synchronous traits, because grading is sequential (a
non-goal) and nothing here has I/O concurrency to exploit. The
private/public split is enforced **by type**: the object handed to the
inner loop cannot score privately.

| Port | Contract | Adapter (`rsi-runtime`) |
|---|---|---|
| `PublicTask` | `baseline()`, `public_score(&Solution, Seed, time limit) -> Result<Attempt>` (score or buggy, feedback, wall time) | `LocalTask`: sandboxed run on the public split, scored in-process. |
| `PrivateGrader` | `private_score(&TaskId, Option<&Solution>, Seed) -> Result<Score>` (no solution or a failed run scores the task's floor) | `SandboxedGrader`: sandboxed run on private inputs, then an out-of-process `rsi __grade` (see §4). It is never constructed on the inner path. |
| `Harness` | `Harness<T: PublicTask>::run(&T, &Budget, Seed) -> Result<InnerOutcome>` (last valid submission, `CostUsage`, encoded transcript, agent log) | `SandboxedHarness`: runs the built agent in the sandbox and serves it over the broker socket (§3). |
| `ChatModel` | `complete(&[Message], max_tokens, timeout) -> Result<Completion>` (text plus prompt and completion tokens) | `OpenAiModel` (plain-HTTP OpenAI-compatible); `ScriptedModel` in tests. |
| `Proposer` | `propose(&[Precedent], &Path) -> Result<Proposal>`: edits the worktree at the path; sees only `Precedent`s (verdicts, grades, public scores), never a per-task private score | `ModelProposer` over any `ChatModel` (so the OpenAI-compatible client); `ScriptedProposer` in tests; `CliProposer` over `codex exec` or Claude Code's `claude -p` in the sandbox (§7, as built). |
| `Executor` | `exec(&SandboxSpec, program, args) -> Result<ExecOutcome>` | `ProcessExecutor`, on Linux: rlimits → Landlock → seccomp → exec, via the `rsi __sandbox` helper. Fails closed where these are unsupported. |
| `CostMeter` | `admit() -> Result<(), BudgetExhausted>`, then `record_tokens` / `observe_wall` | A concrete struct in core, not a trait: there is one implementation, and the broker is its only caller. |
| `LineageStore` | `append(&LineageEntry) -> Result<Digest>` (the entry's chain hash), `entries()` (verifies the chain first) | `JsonlLineage`: append-only JSONL plus a content-addressed blob dir (see §6). |

Each trait lands in `rsi-core` in the same phase as its first adapter
(P2: `PublicTask`, `PrivateGrader`, `Executor`; P3: `Harness`, `ChatModel`; P4:
`Proposer`, `LineageStore`), so no port is defined before something
implements it.

Types in core: `CandidateId`, `Score`, `Grade`, `Budget`, `CostUsage`,
`LineageEntry`, `Decision`, `Seed`, `NoiseBand`, `Margin`. The accept
gate and search helpers are **functions** (`screen`/`confirm`, `ucb1(...)`,
`softmax_sample(...)`), not traits. Each has one call site (the outer
loop's parent selection), so a trait now would be speculative. Core also
carries its own ~20-line SplitMix64, because the workspace has no seeded
PRNG crate: `rusty_rand` is OS-CSPRNG only, and the four private copies
are not exported.

### 3. The inner harness and its broker

`rsi-harness` is a std-only binary. The runtime builds it per candidate
with plain `rustc` in a sandbox, not Cargo:

```
rustc --edition 2021 -O src/main.rs -o <out>
```

Consequences of building with plain `rustc`:

- There is no manifest, `build.rs`, proc-macro, dependency or lockfile
  on the candidate path. A candidate cannot add dependencies or run
  build-time code.
- `harness/Cargo.toml` still makes it a workspace member, so CI builds
  and tests a0. Its manifest is **frozen**: the allowlist is
  `crates/apps/rusty_rsi/harness/src/**`.
- A compile error is a graded **buggy** candidate
  (`Rejection::Buggy`, no evaluations), never a runtime error.

The harness has **no network and no filesystem access beyond its work
dir**. It talks to one Unix socket, the broker, which is owned by
`rsi-runtime`. It uses length-prefixed frames with three operations:

| Op | Effect |
|---|---|
| `llm(messages) -> text` | The broker forwards the call to the configured inner model endpoint and charges tokens (`usage`) to the `CostMeter`. |
| `eval(solution) -> {score \| bug, output}` | The broker runs the solution in a fresh, stricter sandbox on **public** inputs and returns `r_pub`. Solution wall-clock is charged. |
| `submit(solution)` | Records `x̂_t`. The latest submission wins. |

Why a broker rather than letting the harness call the model itself:

- **Budget.** Metering cannot be bypassed.
- **Network.** "No network except the model endpoint" holds by
  construction: seccomp denies `AF_INET*`, and `AF_UNIX` to the broker
  is the only way out.
- **Scoring.** The public scorer is trusted runtime code that the
  harness cannot edit (AIDE² reports reward hacking by AIDE0 on 63% of
  test cases).

**Budget hard stop, identical for every candidate.** Once `tokens ≥
b_tokens` or `wall ≥ b_wall`, every op except `submit` returns
`BudgetExhausted`. At `b_wall + grace` the runtime kills the process
group. If a task has no submission, it scores the task's declared floor
(`task.json: floor`).

`a0` ports AIDE0's behaviour:

- It drafts until there are 5 drafts.
- Then, with p = 0.5, it debugs a random buggy leaf of debug depth ≤ 3;
  otherwise it improves the best non-buggy node.
- Selection is greedy.
- Context is naive: the full history, every attempt's code plus output.
- It submits the best node by public score.
- It is seeded from `--seed` (its own SplitMix64), so it is deterministic
  given the broker's responses.

**As built (P3).**
- **Build.** The runtime copies the candidate's `src/` (directories and
  regular files only, at most 4 MiB; a symlink is a build failure) into a
  fresh build directory and runs, in the sandbox,
  `rustc --edition 2021 -O --crate-type bin --crate-name rsi_harness
  src/lib.rs`. `src/lib.rs` is the binary root and its `pub fn main` the
  entry point. To Cargo the crate is a library with no `main.rs`, so CI
  builds, lints and unit-tests a0 as an ordinary workspace member. The
  build reads only system directories, the toolchain's sysroot (found once,
  outside the sandbox, with `rustc --print sysroot`) and the copy.
- **Channel and socket rules.** The broker socket is one end of a
  `socketpair`, passed as the agent's standard input. The seccomp filter
  (inherited by every descendant) takes a `Sockets` rule:
  - **Agent: `None`.** `socket(2)` and `socketpair(2)` fail with `EPERM`,
    so the inherited socket is the only way out: no network, and no Unix
    or abstract socket either (Landlock covers neither).
  - **Build: `NoEndpoints`.** `socket(2)` fails, so nothing can reach an
    endpoint, but anonymous socketpairs work. rustc starts its linker
    through std's fork-and-exec path, which reports exec errors over an
    `AF_UNIX` socketpair, so refusing it breaks every build. A socketpair
    connects only processes inside the job, and the build runs no
    candidate code (no build scripts or proc-macros).
  - **Solutions: `NoInternet`** (P2, unchanged).
  - **Every sandbox:** `io_uring_setup`, `io_uring_enter` and
    `io_uring_register` fail with `EPERM`, because `io_uring` operations
    (`IORING_OP_SOCKET`, `IORING_OP_CONNECT`, ...) bypass seccomp.
- **Frames.** Length-prefixed binary (`u32` big-endian length, a tag byte,
  then length-prefixed UTF-8 fields), because a std-only agent has no JSON
  parser. `rsi-runtime/src/protocol.rs` and `harness/src/broker.rs` each
  pin the same golden bytes. A malformed or oversized frame ends the
  session; it is not an error.
- **Metering.** `LiveService` charges model tokens from the endpoint's
  `usage` field (a response without it is refused as unmetered) and asks
  for at most min(per-call cap, tokens left). Wall-clock time runs from
  session start and is charged before and after every operation. `submit`
  is free and stays open after exhaustion. The agent process is killed at
  `b_wall + grace`.
- **Deadline.** No model call or evaluation outlives the wall-clock
  budget. `ChatModel::complete` and `PublicTask::public_score` each take
  the time left as a limit. `OpenAiModel` bounds the whole call, not each
  read, so a server that drips bytes cannot stretch it.
  - **DNS is outside every call.** `OpenAiModel` resolves its host once,
    when it is built and before any budget starts.
    - An IP literal needs no lookup.
    - A name is looked up on a resolver thread, which the caller waits on
      for at most 10 s.
    - std's resolver cannot be cancelled, so only one lookup may run at a
      time. A stalled resolver strands one thread, not one per attempt.
  - **Connect is inside the call.** Connecting uses the time left before
    the call's deadline. An evaluation's
  limits are cut to the time left, and its process group is killed at that
  point. A call cut off by the deadline is answered `exhausted`, so the
  run keeps its earlier submission; a call that fails while time remains
  is an endpoint failure.
- **Transcript bound.** The transcript lives in the runtime's memory,
  outside the agent's limits, so it is capped at 64 MiB. Each exchange
  is charged its frames plus 256 bytes, so floods of tiny requests are
  bounded too. A request is served only if a largest-possible exchange
  still fits, so the cut falls at the same request in a replay, and the
  last accepted submission is kept. A response larger than a frame is
  replaced by `refused`.
- **Model failures** (endpoint down, HTTP error, malformed reply) end the
  run with `RuntimeError::Model`; they are infrastructure, never graded.
- **Transcript.** Every exchange, in order, encoded as the same frames.
  `HarnessProcess::replay` serves the recorded responses, fails with
  `RuntimeError::Broker` as soon as the agent sends a request the recording
  did not, hangs up where the recording ends (as the live session ended,
  whether the agent exited or was killed), and returns the replayed final
  submission.
- **Task description.** Each task's `public/task.md` (goal, data formats,
  solution contract, metric) is staged into the agent's work directory.
  The agent never sees data files; it learns about them through `eval`.
- **Models.** `OpenAiModel` speaks OpenAI-compatible `/chat/completions`
  over plain HTTP through `rusty_http`'s sync adapter; `https://` endpoints
  are refused until TLS is wired in. The API key comes from the
  environment and is redacted from `Debug`. With a key configured, nothing
  the endpoint sent reaches a diagnostic, because an endpoint may echo a
  rejected key in full or in part. That covers error bodies and the
  parser errors that quote the response head, its framing (for example a
  `Content-Length` or chunk-size line) or the body. Each diagnostic still
  names its category. Every response framing,
  chunked included, fails as soon as the body passes 16 MiB.
  `ScriptedModel` serves CI.
  `rsi inner` reads `RSI_INNER_MODEL`, `RSI_INNER_BASE_URL` (default:
  local Ollama) and `RSI_INNER_API_KEY`.
- **Tests (`rsi-cli/tests/inner.rs`).**
  - Invariant 2: a0 with 1,200 tokens at 150 per call gets exactly 8 calls,
    and the 8th program's evaluation is refused. An agent that ignores the
    wall clock is killed at `b_wall + grace` and keeps its earlier
    submission.
  - Invariant 4: a run replays from its transcript to the same submission
    without calling the model, and a changed task description diverges.
  - Invariant 1(b): all of these fail with `PermissionDenied`:
    - reads of private labels, public labels, the task directory and the
      repository;
    - a TCP connect, a Unix socket and a socketpair;
    - a connect to an abstract Unix endpoint the test listens on, which
      sees no connection.

    An agent whose sandbox could reach a protected path is not started.
  - Deadline: a model endpoint that accepts and never answers, and an
    evaluation that sleeps for a minute, both end at the 2 s budget. The
    earlier submission is kept, and no solution process survives.
  - A candidate that does not compile is a `BuildFailure`.
- **Unit tests.** A seccomp interpreter checks that the filter refuses
  exactly the listed syscalls under each rule. Other unit tests cover the
  transcript cap (a flood of refused submissions and exhausted calls, with
  a deterministic cut and the submission kept), the model and evaluation
  deadlines, a silent and a dripping endpoint, oversized bodies in every
  framing, and a sentinel key echoed by a 401 (absent from `Display`,
  `Debug` and the CLI's `rsi inner: {error}` line).
- **Mutation check.** Each defence, removed alone, turns its test red:
  - the socket rule, then the socketpair half of it (isolation);
  - the budget admission check (token budget);
  - the transcript cap;
  - the model deadline and the evaluation deadline;
  - the chunked body cap;
  - the key withholding, then its extension to parser errors;
  - the one-lookup guard and the lookup timeout.
- **Known limits.** Solutions (not the agent) can still create Unix
  sockets, as P2 allows. A model call cut off by the deadline is not
  charged tokens, because the endpoint reported none. The token cap is admit-then-record, so the last
  call may overshoot by at most its own prompt tokens (its completion is
  capped at the tokens left).

### 4. Private isolation (invariant 1)

- **Where private data lives.** `private/` stays in the main checkout
  and is never copied into a candidate path.
- **Sparse worktrees.** Candidate worktrees are sparse and contain
  `harness/` only.
- **Landlock read allowlist.** Every sandboxed process (harness build,
  harness, solutions, outer-proposer subprocess) gets an explicit
  Landlock *read* allowlist. We use `platform::Sandbox::confine_filesystem`
  from `platform-linux` (layer `platform`). It is **not** `nexus-security`,
  which always grants read on `/`. The allowlist covers the toolchain and
  system dirs, the candidate's worktree or binary, a run-scoped copy of
  `public/`, and its own work dir. The repository root, the run's lineage
  dir and every `private/` are outside the allowlist.
- **Private scoring, out-of-process.** After the harness exits, the
  runtime runs `x̂_t` in a *fresh* sandbox on private **inputs** only.
  It then hands the outputs to `rsi __grade`, a separate, unsandboxed
  process, the only one that reads private **labels**.
- **Required tests.**
  - (a) A solution that opens a known private-label path gets `EACCES`.
  - (b) Likewise for the harness.
  - (c) With Landlock unavailable, the executor refuses to run rather
    than running unconfined.

**As built (P2).**
- **Solution runs.** A solution runs as `python3 solution.py
  data/<inputs> output.txt` in a fresh work directory. That directory holds
  the solution, the task's shared public files and one split's inputs,
  staged by the runtime. The task directory itself is never in the sandbox.
  - Before every run, `SolutionRunner` checks
    `SandboxSpec::can_reach(task root)` and refuses if the sandbox could
    reach it.
  - Read roots are `/usr`, `/lib`, `/lib64` and `/bin`, canonicalised.
    `/etc` is excluded: Python needs none of it.
- **The helper.** The sandbox helper is the `rsi` binary itself
  (`rsi __sandbox`), single-threaded from birth. It applies these steps in
  order, then `exec`s the program:
  1. rlimits: CPU (soft, plus a hard limit 1 s later), address space,
     file size, descriptors, processes, and core dumps set to 0;
  2. Landlock;
  3. seccomp.
- **Fail closed.** A step that is not `Enforced` is a setup error. The
  helper writes it to a status file that the parent created and the helper
  opened *before* confining itself. The descriptor is close-on-exec, so the
  file stays empty after a successful `exec`, and the untrusted program can
  neither reach it nor forge it. Any non-empty status file aborts the run
  with `RuntimeError::Sandbox`.
- **Wall-clock limit.** The executor enforces it by `SIGKILL` to the whole
  process group, and kills the group again after every exit so stragglers
  die too.
- **Tests.**
  - (a) is `a_solution_cannot_read_private_*` (public scoring and private
    grading).
  - (c) is covered two ways. `sandbox_setup_failure_fails_closed` uses an
    un-addable Landlock root, so the program never runs. The
    `require_enforced` unit test covers `NotEnforced` and `Unsupported`.
    Off Linux, `execution_fails_closed_off_linux` runs on CI's Windows job.
  - (b) is `the_agent_cannot_reach_private_data_the_repository_or_any_socket` (P3).
- **Mutation check.** With the Landlock and seccomp step removed, exactly
  the five confinement tests fail. With the rlimits removed, the memory and
  CPU tests fail.
- **Untrusted output (review of #476).** The parent reads `output.txt`
  outside the sandbox, so the file is treated as hostile.
  `output::read_untrusted` opens it once with `O_NOFOLLOW | O_NONBLOCK`. It
  accepts only a regular file with one link and reads at most the task's
  file-size limit from that descriptor.
  - A symlink to private labels, a FIFO, a device or a hard link scores
    the floor promptly, and a path swapped after the open has no effect.
  - The bytes read are the only snapshot: private grading sends them to
    `rsi __grade` on stdin (`--output -`) and never reopens the path.
  - Public scoring reads inputs from the task directory, never from the
    work dir the solution controlled.
- **Job containment (review of #476).** A process can leave its process
  group only through `setsid` or `setpgid`. A second seccomp filter, stacked
  on the socket filter, makes both fail with `EPERM` (and refuses x32-ABI
  syscalls), so the process group *is* the job.
  - After every exit or timeout the executor `SIGKILL`s the group, then
    scans `/proc` until no live member remains.
  - If any member survives 2 s of that, the run fails closed
    (`RuntimeError::Sandbox`).
  - Cost: `subprocess.Popen(..., start_new_session=True)` and similar
    raise `PermissionError` inside the sandbox.
  - This stays within this ADR's scope: no cgroups, no namespaces,
    unprivileged, and it works on CI's kernel.
- **Known limit.** `RLIMIT_NPROC` counts every process and thread of the
  user, not just the sandboxed tree, and the kernel does not apply it to
  root.
  - It is therefore set high: `PROCESS_LIMIT` = 4096. CI showed why. At 64,
    `subprocess.Popen` failed with `EAGAIN` because the runner's
    (non-root) user already had that many threads; locally the run was
    root and unaffected.
  - It is a fork-bomb brake, not a quota. The wall-clock kill of the
    process group is what bounds a run, which is one more reason cgroups
    stay listed under Out of scope.

### 5. Budget, noise and the accept gate (invariants 2, 3)

- **Units.** `Budget { tokens: u64, wall: Duration, gpu_seconds:
  Option<f64> }` per task. It is the same for every candidate and is
  recorded in every lineage entry.
- **Calibration.** `rsi calibrate` grades a0 N = 5 times on disjoint
  seed sets and computes `σ̂` = the sample standard deviation of the 5
  grades. It reports the noise band (min, max, mean, σ̂) and writes
  `margin = z · √2 · σ̂`, with z = 1.645 by default (one-sided 95% on
  the difference of two independent grades). That value goes to the run
  config. With N = 5, `σ̂` is itself imprecise; the report says so, and
  `z` and `N` are config.
- **Gate.** Let `g_inc` be the incumbent's grade.
  - A candidate graded on seed set S₁ with `g₁ ≤ g_inc` is rejected.
  - Otherwise it is re-graded on fresh, disjoint seeds S₂. It is
    accepted iff `mean(S₂) − g_inc > margin`.
  - Only the fresh mean is compared, which avoids the winner's curse of
    reusing the S₁ result that triggered re-evaluation.
  - Seeds come from a counter-based derivation (`SplitMix64(run_seed,
    candidate, round)`), so "fresh" is checkable in lineage.

### 6. Lineage (invariant 4)

`runs/<run-id>/lineage.jsonl` is append-only:

- The file is opened `O_APPEND`, and the store has no update or delete
  API.
- Each line carries `prev_sha256` (via `rusty_rsa::sha256`). `verify()`
  walks the hash chain; any edit or truncation in the middle fails it.

Large values live in `runs/<run-id>/blobs/<sha256>`: diffs, chosen
solutions, full broker transcripts. Every entry records the following:

- harness commit SHA, parent SHA, diff blob
- seeds per task
- inner and outer model ids plus endpoint kinds
- `Budget` and `CostUsage` (tokens, wall, gpu)
- per-task public score of `x̂_t`, private score, and grade
- the accept decision with the margin used
- a host fingerprint (wall-clock budgets depend on the machine)

**Evidence binding.** A `LineageEntry` can only be built by
`LineageEntry::new`, which re-runs the accept gate (`screen`, then
`confirm`) on the entry's recorded evaluations with the recorded incumbent
grade and margin. It refuses the entry unless the result equals the
recorded decision. A gate verdict therefore cannot carry a grade the
evidence does not produce, and a duplicated `(task, seed)` result is an
error rather than extra weight. Verdicts for candidates that were never
graded (`Buggy`, `PathViolation`) must have no evaluations.

**Replay** (`rsi report --replay`) works at two levels:

- *Grade replay.* It re-runs `rsi __grade` on the stored `x̂_t` blobs
  and must reproduce each private score bit-for-bit.
- *Trajectory replay.* It re-runs the candidate harness against the
  recorded broker transcript: model replies are served from the blob,
  not the model. It must produce the same `x̂_t`.

That is how "deterministic replay" holds despite a non-deterministic
LLM. A test covers both levels.

Candidate commits live under `refs/rsi/<run-id>/<n>` (detached
worktrees), not branches, so runs never clutter `git branch`.
Worktrees are removed after grading, and the ref keeps the commit.

### 7. Outer loop

For step k:

1. The outer loop selects a parent. The default is the incumbent, which
   is AIDE²'s rule; `ucb1` and `softmax` over accepted entries are
   config.
2. It creates a sparse worktree at the parent.
3. It calls `Proposer` with prior entries. These include **grades only**,
   never per-task private scores or private outputs, plus inner-run
   public transcripts.
4. It commits the result and checks that the changed paths (`git diff
   --name-only` plus untracked files, symlinks rejected) are all under
   the allowlist. A path outside it gives
   `Rejection::PathViolation` and is recorded, not applied.
5. It builds, grades, gates and appends.

Both the `CodexCliProposer` subprocess and the HTTP proposer run under
the same Landlock read allowlist. Network is allowed there, since that
is the outer model's endpoint. (As built, the HTTP proposer runs in the
`rsi` process and only `codex exec` is sandboxed; see below.)

**As built (P4).**
- **Ports.** `Proposer::propose(&[Precedent], &Path)` edits the worktree in
  place. `rsi_core::precedents` is the only way to build its input: per
  candidate, the verdict, the first-round grade, a build or path note, and
  the first round's public scores. A per-task private score has no field
  to live in. Inner-run transcripts are not passed (they are large); the
  public scores stand in for them.
- **Worktrees.** `git worktree add --detach --no-checkout`, then
  `sparse-checkout set --no-cone /crates/apps/rusty_rsi/harness/`, then
  `checkout`, so the proposer sees only the harness crate.
  - `git add --all --sparse` stages everything, including files created
    outside the sparse checkout, without mistaking unchecked-out files for
    deletions.
  - `git diff --cached --raw -z --no-renames` lists each path with its new
    mode.
  - The allowlist accepts only regular files (modes `100644`, `100755`,
    or a deletion) under `crates/apps/rusty_rsi/harness/src/`.
  - A violation is committed (for the audit trail) and recorded as
    `PathViolation`, and never built.
  - Every git call runs with hooks disabled (`core.hooksPath=/dev/null`),
    no signing and a fixed identity.
- **Refs and run names.** Candidates are committed under
  `refs/rsi/<run>/<step>`, where `<run>` is the run directory's name
  (lowercase, digits, `-`, `_`). `rusty_uuid` is not used: an explicit
  directory is simpler to find and to reason about.
  - **Claims.** Every ref is created with `git update-ref --stdin`
    `create`, which fails if the ref exists. A run first claims
    `refs/rsi/<run>/base`, before writing anything, so a second run
    directory with the same name is refused and the first run's refs and
    candidates are never overwritten.
- **Loop.** The parent is always the incumbent (AIDE²'s rule). `ucb1` and
  `softmax` selection stay in core, unused until a run needs them.
  - **Each step:** propose, stage, commit, check the allowlist, build
    (a compile error is `Buggy`, with the first 4,000 characters of the
    compiler output), grade round 0, `screen`, grade round 1 on a fresh
    seed set, `confirm`, append.
  - **The incumbent's grade** after an acceptance is the fresh grade.
  - **Infrastructure failures** end the run; entries already appended
    stay valid.
- **Grading.**
  - **Runs.** Each round runs every task once per seed
    (`seed_set(run_seed, candidate, round, n)`), all under the same budget.
  - **Recorded per task:** the submission's public score, its private
    score (the floor if there was no submission), the submission and
    transcript blobs, and the inner cost.
- **Lineage file.** Each line is
  `{"hash":"<64 hex>","prev":"<64 hex>","entry":<entry>}`.
  - **Exact bytes.** The fixed-width prefix lets a reader recover the
    entry's exact bytes, which `hash` covers.
  - **Encoding.** Scores, grades, margins and 64-bit seeds are decimal
    strings, which round-trip exactly; durations are integer nanoseconds.
  - **Validation.** Reading re-runs `LineageEntry::new`, so a recorded
    verdict must still follow from its evidence.
  - **Create vs open.** `JsonlLineage::create` makes a new, empty
    lineage and fails if one exists; `open` fails if there is none, so a
    missing lineage is never read as an empty run.
  - **Completeness.** `report::check` requires a baseline first, then
    candidates in step order, at most `steps` of them. A run without a
    baseline has nothing to report or reproduce. A valid prefix is
    reported as `INCOMPLETE, k of n`, and `--replay` on it fails after
    replaying what was recorded. The hash chain does not detect a
    truncated suffix; the step count in `run.json` does, for the tail.
  - **Appends** use `O_APPEND` plus `fsync`. **Blobs** are written aside
    and renamed, and checked against their address on every read.
- **Configuration.** Command-line flags plus a calibration JSON file
  (`rsi calibrate --out`, `rsi run --calibration`), not `rsi.toml`: the
  workspace has no first-party TOML parser, as with `task.json`. Each
  role reads `RSI_<ROLE>_MODEL`, `_BASE_URL` and `_API_KEY`, with `INNER`
  for the agent and `OUTER` for the proposer. `run.json` records the
  model ids, never a key.
- **Proposer.** `ModelProposer` sends the system contract, the
  precedents and the current harness source.
  - **Reply format.** The model replies with whole files as
    `<<<FILE path` ... `>>>END`.
  - **Writing.** Files are written only inside the worktree: absolute
    paths, `..` and `.git` (in any case) are refused, every ancestor
    directory must be a real directory (a symlinked parent is refused,
    and missing ones are created one level at a time), and a planted
    symlink at the leaf is replaced rather than followed. The allowlist decides afterwards.
- **Codex CLI proposer** (`CliProposer` with `CliAgent::Codex`, `RSI_OUTER_PROVIDER=codex`;
  `rsi-runtime::agent_cli`).
  It runs `codex exec` under the sandbox helper, in its own profile.
  - **Why our sandbox only.** Codex's Linux sandbox (bubblewrap) cannot
    start inside a Landlock domain ("error building bubblewrap command:
    Permission denied" with codex-cli 0.160.0), and running Codex outside
    ours would let the commands it runs read private task data, since its
    own `workspace-write` mode reads the whole disk. So Codex runs with
    `--dangerously-bypass-approvals-and-sandbox` (its documented mode for
    an external sandbox), plus `--ephemeral`, `--ignore-user-config`,
    `--ignore-rules` and `--skip-git-repo-check`, and this sandbox is the
    boundary.
  - **Files.** Codex edits a staging copy of the worktree with no `.git`,
    so it cannot redirect the commit. It may write only that copy, its
    `CODEX_HOME`, a private `TMPDIR` and `/dev/null`. It may read the
    system directories, `/etc` (DNS and certificates), `/dev/urandom`,
    its install directory and the directory of `SSL_CERT_FILE`. The
    proposer refuses to start if that spec could reach a protected path
    (the tasks, the repository, the run directory, the executor's state).
    File roots such as `/dev/null` need rustils 0.27.2, whose Landlock
    rules accept a single file.
  - **Network.** A new socket rule, `Sockets::Internet`, skips the
    internet-socket filter; the process-group, `io_uring` and x32 locks
    stay. Proxy and certificate variables (`HTTPS_PROXY`, `NO_PROXY`,
    `SSL_CERT_FILE`, ...) pass through; nothing else from the
    environment does.
  - **Back into the worktree.** Each change in the copy is replayed with
    `write_inside`, `link_inside` or `remove_inside`, which keep the
    no-follow, no-`.git` rules above, so a symlink Codex makes is
    recreated for the allowlist to reject, never followed.
  - **Login and cost.** Codex signs in with its own login under
    `CODEX_HOME`; no key passes through `rsi`. Its token use is not
    reported back, so the proposal's usage is zero (the outer cost was
    already unrecorded).
  - **Tests.** A fake `codex` script runs through the real helper and
    checks the edits that come back, that `.git` is untouched, and that
    it cannot write outside, read private data or open devices other
    than `/dev/null` and `/dev/urandom`, while internet sockets work. An
    `#[ignore]`d test runs a real, logged-in Codex.
- **Claude Code proposer** (`CliAgent::Claude`, `RSI_OUTER_PROVIDER=claude`).
  The same sandbox, staging copy and mirror as Codex, with
  `CLAUDE_CONFIG_DIR` as its home. Claude Code needs no bypass flag: it
  runs `claude -p --restricted` with the file tools only
  (`Read,Edit,Write,Glob,Grep`, so nothing that runs commands),
  `--permission-mode acceptEdits`, `--permission-prompts none`, no settings
  sources, MCP servers, slash commands or session files, and at most 60
  turns. Its reply is the `result` of the `--output-format json`
  envelope; an envelope marked `is_error` is an error. It signs in with
  the Claude subscription login; no Anthropic key passes through `rsi`.
- **Codex as the inner model** (`CodexModel`, `RSI_INNER_PROVIDER=codex`;
  `rsi-runtime::codex_model`). a0 stays the agent under improvement and
  the broker still meters and records every call; only the completion
  comes from Codex.
  - **Why not Codex (or Claude Code) as the inner agent.** It would
    replace a0, leaving the outer loop nothing to improve, and its own
    agent loop would call its model around the broker, breaking the
    budget hard stop (invariant 2) and trajectory replay (invariant 4).
  - **Each call** runs one `codex exec --json` in a fresh, empty staging
    directory under the proposer's sandbox, with the messages rendered
    as one prompt under their roles and an instruction to answer with
    text only. It is killed at the timeout the broker passes.
  - **Metering.** Tokens come from the `turn.completed` events' `usage`
    (`input_tokens`, `output_tokens`); a `turn.failed` event or a run
    with no usage is an error, never an unmetered call. The executor
    keeps 8 MiB of the event stream for it. Codex has no per-call output
    cap, so one call may overrun the remaining budget; the meter charges
    it and the broker refuses the next call.
- **Replay** (`rsi report --replay`).
  - **Grade replay.** Every stored submission is re-graded through the
    out-of-process grader, and each private score must match bit for bit.
  - **Trajectory replay.** Every graded candidate is rebuilt from its
    commit and re-run against each recorded transcript; the same
    submission must come out.
  - **Failures.** A missing or altered blob is an error; a disagreement
    is listed, and the command fails.
- **Calibration.** `rsi calibrate` grades the base harness on N rounds
  (default 5) of candidate 0 and writes the band and
  `margin = z · √2 · σ̂`. It warns that σ̂ from N = 5 is rough.
- **Tests (`rsi-cli/tests/outer.rs`).** A 10-step run in a throwaway repo
  holding a copy of the harness, on the real three-task suite, with the
  scripted inner model and scripted proposals of known strength. Each
  proposal gets its verdict:
  - not better (equal);
  - within noise (+0.20 against a 0.30 margin);
  - path violation (the harness manifest);
  - buggy;
  - accepted (+0.61 on a fresh, disjoint seed set);
  - not better (after the incumbent changed);
  - path violation (a symlink in `src/`);
  - not better (no change);
  - path violation (a task's private labels);
  - not better (identical to the incumbent).

  The test also checks:
  - parents and refs, and that no new branches were created;
  - that each proposal saw the lineage so far;
  - that the stored lineage equals the returned one;
  - that replay reproduces all 27 private scores and 27 trajectories;
  - that an altered submission blob and a forged verdict are both caught;
  - that a finished run cannot be overwritten.

  A second test calibrates on three rounds.
- **Mutation check.** Each of these, removed alone, fails the run test:
  - the allowlist;
  - fresh seeds for the re-evaluation;
  - passing the full history to the proposer.
- **Known limits.**
  - `run.json` keeps the run seed. Calibrating and running with the same
    `--seed` makes calibration's round 0 and the baseline share seeds,
    which is harmless: neither is a gate decision.
  - Cost of the outer model is not yet recorded in lineage.

### 8. Models and configuration

- Inner model: any OpenAI-compatible `/v1/chat/completions` over plain
  HTTP, via `rusty_http`'s sync transport. The default is Ollama at
  `http://127.0.0.1:11434/v1`. Token usage comes from its `usage` field.
- Outer model: the same client, or `codex exec` or Claude Code's
  `claude -p` as a sandboxed subprocess with its own subscription login.
  There is no Anthropic key path.
- Inner model alternative: `codex exec` per call (`CodexModel`), metered
  from its event stream.
- `rusty_llama`'s OpenAI-compatible `server` feature can serve as a
  fully in-process, offline inner model later, with zero adapter code.
- Config: `rsi.toml`, with explicit env overrides (`RSI_*`). Secrets
  come only from env (e.g. `RSI_OUTER_API_KEY`) and are never written to
  lineage or logs; lineage records the endpoint kind and model id only.
- CI has no model. Tests use a scripted model behind the same broker,
  and scripted proposers.

## Alternatives considered

- **Extend `rusty_skillopt`.** Rejected on its call sites, not on taste.
  Every one of them is skill-text-specific:
  - `Engine::run_step` hard-codes the skill system prompt
    (`engine.rs:52-57`).
  - `apply_edit(&Skill, &SkillEdit)` edits line-anchored markdown
    (`skill_edit.rs:20`).
  - `ChatBackend::chat` returns `String` and drops token usage
    (`traits.rs:13`; `openai_compat.rs` never reads `usage`).
  - The gate is a fixed scalar with no variance model:
    `val > best + min_improvement` (`engine.rs:246`).
  - The core is async-trait-based.

  Generalising it means rewriting `engine.rs`. It is also a
  cross-family `apps → apps` edge, which CI rejects.
- **Extract a shared optimiser core from `skillopt` now.** There is no
  second call site until `rusty_rsi` exists and stabilises. Revisit when
  both loops are in `main`. The only shared shape is "gate a candidate
  on held-out score", which is ~10 lines.
- **Depend on `rp-router` for cost metering.** Its budgets are USD per
  client per calendar period (`client_budget.rs:14,30`), and its
  persistence stores aggregate counters, not events. It pulls in
  tokio, tokio-postgres, prometheus and rustls, and it is `apps`.
- **Use `rp-providers` for model calls.** Moving `rp-core` and
  `rp-providers` to `libs/` (ADR-0003 anticipates this) edits another
  family's paths in root `Cargo.toml`. Their `Provider` is async over
  reqwest/tokio, which we do not need. A ~150-line sync client over
  `rusty_http` suffices.
- **`rusty_adk` or `rk-kernel` as the agent loop.** AIDE0 is a tree
  search over single completions, not a tool-calling loop. The inner
  loop is also the thing being rewritten, so it must stay small and
  std-only. `rk-kernel` is `apps`, and its loop is private to `aisdk`.
- **`nexus-security` for the sandbox.** Read access to `/` is always
  granted (`os_sandbox.rs:286-307`), which defeats private isolation. It
  is also `apps`.
- **Reuse `sessionmgr-git`.** It is `apps`, and it lacks commit, sparse
  checkout and untracked-in-diff. `rsi-runtime` gets its own small
  git-CLI adapter. That makes two call sites, so hoisting a shared
  `libs/` git-CLI crate is a **follow-up needing sign-off**, since it
  touches `rusty_yirp`.
- **`rusty_sqlite` for lineage.** It fits, but the benefits don't apply:
  about 100 rows per run need no SQL, and JSONL is diffable and
  greppable. It would add a Tier-A dependency and the `links =
  "sqlite3"` pin to a family that otherwise has none. `rusty_multimodal_db_engine`'s
  `insert_log` is cleared at compaction and has no hash chain.
- **Cargo-built harness with a normal manifest.** A candidate could
  then add dependencies or a `build.rs`, which is arbitrary code at
  build time. Edits would also touch the root `Cargo.lock`, outside the
  allowlist.

## Out of scope

- Weight training and GPU-kernel tasks.
- Distributed or parallel grading, and a UI.
- Hermes/Agent OS integration, and running the ignition test.
- cgroups: the MVP uses rlimits (`RLIMIT_AS`, `RLIMIT_CPU`,
  `RLIMIT_FSIZE`, `RLIMIT_NOFILE`, `RLIMIT_NPROC`) plus a wall-clock
  kill of the process group. `RLIMIT_NPROC` is per-uid, so it is a
  fork-bomb brake, not a quota.
- Non-Linux executors: the executor fails closed on other targets.
- HTTPS for OpenAI-compatible endpoints; remote outer models use the
  Codex CLI.

## Consequences

- There are four new workspace members; root `Cargo.toml` gains only
  `members` entries. `docs/WORKSPACE-MAP.md`, README's crate table,
  `RELEASE_NOTES.md` and `CHANGELOG.md` are updated per PR.
- Tier by ADR-0002: `rsi-core`, `rsi-runtime`, `rsi-cli` and
  `rsi-harness` are all planned **Tier S**, with no external
  dependencies. A crate that needs one goes to Tier A with a stated
  reason.
- **Host requirements for `rsi run`:**
  - Linux with Landlock (ABI ≥ 1).
  - `rustc` on PATH.
  - `python3` (the toy tasks' solution runtime; stdlib only).
  - `git` ≥ 2.25 (sparse checkout).
- Sandbox tests need Landlock in CI. If the runner kernel lacks it, the
  fail-closed test still runs and passes. The EACCES tests need
  Landlock, so a runner without it is a CI configuration change for
  sign-off, never a skip.
- The harness's source is expected to grow and pick up dead code under
  the outer loop (the report says so of AIDE85). Candidate code lives
  only under `refs/rsi/*`. Promoting an evolved harness into `main` is
  an ordinary reviewed PR.

## Appendix A: reuse matrix

| Capability | Existing crate (layer) | Fit | Decision |
|---|---|---|---|
| Optimiser loop and gate | `skillopt-core` (apps) | Skill-text-specific, fixed-margin gate, async | Build new; gate is noise-aware (§5) |
| Cost meter | `rp-router` (apps) | USD per client per period, aggregates, heavy | Build new: tokens + wall in core |
| Token usage type | `rp-core` (apps) | `Usage{prompt,completion,total}` matches | Mirror 3 fields; no dependency (layer rule) |
| Model adapter | `rp-providers` (apps) | Async reqwest; no Ollama-specific adapter | Sync OpenAI-compatible client over `rusty_http` (libs) |
| HTTP/1.1 | `rusty_http` (libs, sync transport, zero default deps) | Plain-HTTP localhost Ollama | **Reuse** |
| In-process model | `rusty_llama` (libs) | Offline OpenAI-compatible server | Later: config only, no code |
| Agent loop | `rk-kernel` (apps), `rusty_adk` (libs) | Tool-calling loops, tokio; AIDE0 is tree search | Not reused |
| Worktrees and diffs | `sessionmgr-git` (apps) | Worktree add/remove, changed files; no commit or sparse | Own adapter; hoist to `libs/` as a follow-up |
| Git object model | `rusty_git` (libs) | No worktrees or packfiles | Not reused |
| Unified diff | `rusty_diff` (foundation) | Single-file, whole-file hunk | Not needed; git produces diffs |
| FS and net sandbox | `platform-linux` `LinuxSandbox` (platform) | Read and write allowlists, inet seccomp | **Reuse** |
| FS and net sandbox | `nexus-security` (apps) | `/` always readable | Rejected |
| rlimits | `rusty_libc::rlimit` (foundation) | `setrlimit`, `RLIMIT_*` | **Reuse** |
| Process spawn contract | `portable-runtime` `contract`/`compat` (platform) | No timeout, kill or limits | Not reused; the executor needs a pre-exec hook |
| Namespaces | `rk-feed` bwrap wrapper (apps) | External binary, apps | Not reused (seccomp covers net) |
| Lineage store | `rusty_sqlite` (libs, Tier A) | Good, but unneeded at this scale | JSONL + sha256 chain |
| Lineage store | `rusty_multimodal_db_engine` (libs) | Transient logs | Rejected |
| Errors | `rusty_err` (foundation) | enum derive; no `std::error::Error` impl | **Reuse** |
| Serialisation | `rusty_serde` + `rusty_serde::json` (foundation) | Derive on structs (used by rusty-search) | **Reuse** |
| Hashing | `rusty_rsa::sha256` (foundation) | Streaming SHA-256 | **Reuse** (local hex) |
| Ids | `rusty_uuid` (foundation, v4 only) | Run ids | **Reuse** |
| Time | `rusty_time` (no `now()`), `std::time` | RFC3339 formatting | `std::time` + `rusty_time` formatting |
| Seeded PRNG | none exported | none | SplitMix64 in `rsi-core` |

## Appendix B: phase plan (one PR per phase, not merged without sign-off)

| Phase | Deliverable | Invariant tests landing |
|---|---|---|
| P1 | `rsi-core`: types, accept gate, noise margin, UCB1, softmax, SplitMix64, `CostMeter`, lineage types and hash chain (pure) | 2 (meter hard-stop boundaries), 3 (gate), 4 (chain verification, pure part) |
| P2 | Task format; 3 toy tasks (ML-lite regression, heuristic TSP, prompt/harness scaffold around a deterministic weak-solver simulator); executor; out-of-process grader; `rsi-cli` with the internal `__sandbox`/`__grade` entry points (the helper and grader must be a binary) | 1, 5 |
| P3 | Broker, `rsi-harness` a0, inner `Harness` adapter, model client, scripted model | 2 (end-to-end hard stop), 4 (trajectory replay) |
| P4 | Outer loop, git adapter, path allowlist, `rsi calibrate`, a 10-step run, `rsi report` | 3 (end to end), 4 (grade replay), path-violation rejection |
