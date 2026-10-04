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
| `rsi-runtime` | `crates/rsi-runtime` | Adapters for every port. | `rsi-core`, `rusty_err`, `rusty_json` (no default features); on Linux `platform`, `platform-linux` (Landlock/seccomp), `rusty_libc` (rlimits, `killpg`); `rusty_http` from P3 → none |
| `rsi-cli` | `crates/rsi-cli` | Composition root: `rsi run \| calibrate \| report`, plus hidden `rsi __sandbox` / `rsi __grade` entry points. | the two above |
| `rsi-harness` | `harness/` | **The only mutable surface**: the inner agent. It has zero dependencies (std only). | none |

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
| `PublicTask` | `baseline()`, `public_score(&Solution, Seed) -> Result<Attempt>` (score or buggy, feedback, wall time) | `LocalTask`: sandboxed run on the public split, scored in-process. |
| `PrivateGrader` | `private_score(&TaskId, Option<&Solution>, Seed) -> Result<Score>` (no solution or a failed run scores the task's floor) | `SandboxedGrader`: sandboxed run on private inputs, then an out-of-process `rsi __grade` (see §4). It is never constructed on the inner path. |
| `Harness` | `run(&PublicTaskView, &Budget, Seed) -> Result<InnerOutcome>` (chosen solution, `CostUsage`, transcript id) | Builds the candidate, spawns it in the sandbox, and serves it over a broker socket. |
| `Proposer` | `propose(&[LineageEntry], &Workspace) -> Result<Proposal>` | `OpenAiCompatProposer` (HTTP) and `CodexCliProposer` (subprocess); `ScriptedProposer` in tests. |
| `Executor` | `exec(&SandboxSpec, program, args) -> Result<ExecOutcome>` | `ProcessExecutor`, on Linux: rlimits → Landlock → seccomp → exec, via the `rsi __sandbox` helper. Fails closed where these are unsupported. |
| `CostMeter` | `admit() -> Result<(), BudgetExhausted>`, then `record_tokens` / `observe_wall` | A concrete struct in core, not a trait: there is one implementation, and the broker is its only caller. |
| `LineageStore` | `append(LineageEntry) -> Result<EntryId>`, `iter()`, `verify()` | Append-only JSONL plus a content-addressed blob dir (see §6). |

Each trait lands in `rsi-core` in the same phase as its first adapter
(P2: `PublicTask`, `PrivateGrader`, `Executor`; P3: `Harness`; P4:
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
  - (b) needs the harness and lands in P3.
- **Mutation check.** With the Landlock and seccomp step removed, exactly
  the five confinement tests fail. With the rlimits removed, the memory and
  CPU tests fail.
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
is the outer model's endpoint.

### 8. Models and configuration

- Inner model: any OpenAI-compatible `/v1/chat/completions` over plain
  HTTP, via `rusty_http`'s sync transport. The default is Ollama at
  `http://127.0.0.1:11434/v1`. Token usage comes from its `usage` field.
- Outer model: the same client, or `codex exec` as a subprocess. There
  is no Anthropic key path.
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
