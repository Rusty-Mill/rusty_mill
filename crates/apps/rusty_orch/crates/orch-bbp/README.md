# orch-bbp

A synchronous `orch_dispatch::TaskHost` adapter over the existing `bbp`
CLI. BBP continues to own the protocol, event store, MCP server, runner,
turn scheduling and moderator. This crate never links `rusty_bbp_host`.

Enable hosting with `rusty_orch run goal.json --bbp bbp.json --state .orch`.
`ORCH_BBP` supplies the same configuration-file path. Alternatively place the
configuration object under `bbp` in the goal file. Without either, existing
research/design/triage/review and agent workflows continue unchanged.
Hosting requires `--state`; keep the BBP store, runner work, and state outside
of the source checkout and outside workload read roots.

The configuration contains:

- `binary`: built `bbp` binary (or `bbp` on PATH).
- `dir`, `repo`, `work`: BBP store, source Git repository, runner work directory.
- `profiles`: existing BBP ProfileSet JSON file, including commands, read roots,
  explicit workload environment and resource limits. For Cargo, include the
  toolchain and cached dependencies required by the offline sandbox. No
  environment or credentials are automatically copied into the workload.
- `pool`: any number of named agents, each with `vendor`, `program`, `args`,
  and optionally `sandbox: {read, write, env, checkout}`, which runs that member's
  harness confined away from the BBP directory (see BBP's "Confined harnesses").
  Launchers must speak BBP MCP. BBP expands `{bbp}`, `{dir}`, `{task}`, `{role}`,
  `{principal}`, `{turn}` and `{mcp_config}`; Orch does not render agent prompts.
  Codex, Claude, Ollama and future providers use the same configuration surface.
- `roster`: BBP v0.5 capability records, not vendor names or fixed roles.
- `assignments`: roster role id to `{agent: POOL_MEMBER, builtin_tools: BOOL}`.
- `live_turns`: positive concurrent turn cap within the roster size.
- `max_wall_secs`: positive lifetime bound for each moderator invocation.
- `slots` (optional): `{dir, agents, validators, ceiling, priority_secs}`,
  machine-wide admission passed to `bbp mod --slots`. Every moderator naming
  the same `dir`, across goals, runs at most `agents` harnesses and
  `validators` runners at once. Waiting turns and runs queue fairly
  ([ADR-0018](../../docs/adr/0018-fair-admission.md)). `ceiling` (optional)
  caps how many agent slots this goal's tasks hold at once, all together.
  `priority_secs` (default 0, at most a day) ranks this goal's waiters as if
  they had arrived that many seconds earlier. With a lease, a task does not
  start a harness while another task whose lease write path overlaps its own
  holds a slot or waits ahead of it. Both values are frozen with the task.
- `lease` (optional): `{read, write, ttl_secs}`, the permission lease Orch
  grants the goal's assignment ([ADR-0017](../../docs/adr/0017-permission-leases.md)).
  Lease paths must exist and be canonical. Every assigned pool member must
  then be sandboxed; its sandbox paths are met with the lease's when the task
  is first frozen, and a pool where one member's path lies beneath any
  member's write root is refused. A later run under a lease that reaches
  other paths is refused, so widening access needs a new task. Each run
  renews the end, which BBP enforces as an absolute deadline on every harness.

See the [two-role configuration](../../docs/examples/bbp.json) and BBP's
[proving configuration](../../../rusty_bbp_host/docs/proving/agents.json).
The two-role example uses `codex-bbp-harness` and `claude-bbp-harness`, built
by `cargo build -p rusty_bbp_host`. Put all three host binaries on PATH, or
use absolute paths. Authenticate the installed Codex and Claude CLIs with their
subscription logins first. Optional launcher `--model MODEL` pins a provider
model; `--provider-bin PATH` selects an installed provider executable. No other
provider flags are forwarded. See the [launcher contract](../../../rusty_bbp_host/docs/provider-launchers.md).
Existing Orch one-call adapters are not BBP harnesses; raw `ollama run` alone
does not implement MCP. Other providers still use explicit custom launchers.

Final reviewers must use scoped message reads, no write capabilities, no raw
logs and no built-in tools. Their configured pool member and vendor must differ
from every candidate writer. BBP starts fresh harnesses per turn and enforces
roster visibility. This is configuration policy; Orch cannot attest what a
user-supplied launcher actually executes. No provider credentials are required
for the scripted tests.

## Durable lifecycle

The task id hashes the goal bytes and goal/card ids. Identical goal bytes in the same BBP store
intentionally adopt the same task; use a separate store for an independent repeat. Before any host command,
Orch publishes a private, no-clobber manifest under `dir/orch/<task hash>`.
It freezes the brief, pool, roster, assignments and profile bytes. Adoption
finishes missing assignments and refuses mismatched existing tasks; it never
reassigns an active role. External configuration files may change between tasks;
an already frozen task keeps its original configuration. Inline changes alter
the goal fingerprint and require fresh Orch state, as with existing goals.

The dispatcher persists one opaque host pointer with the plan and ledger through
`orch-store`. A successful open/adopt is charged once; failed or uncertain opens
are retried through that id without charging another hosted card. Polls, human
responses, and resumed tasks consume no additional Orch calls. BBP meters its
own internal turns and resource budgets. `waiting` exits 0: invoke Orch again to
poll; it does not install a polling daemon.

Human gates carry the exact displayed revision and subject ids in Board refs.
Only human-authored answers are relayed. Answers are BBP verb lines, for example
`approve-plan art:2`, `approve-merge art:4 run:1`, or `receipt art:4 COMMIT`.
Destructive commands also carry the saved revision. All verbs get an additional
transactional revision fence and a stable operation id; crash retries replay
successful answers without applying them twice. Stale answers produce a new
question. Flags in answer text and subjects outside the displayed gate are
refused. Invalid host/protocol responses surface as failures; they are never
accepted solely because the process exited zero.

Approval is a receipt gate, not completion. Orch completes only on BBP `Closed`
with its recorded receipt, retaining candidate and merged revision artifacts.
Cancellation fails the card with its reason. Pending requests and escalations
also become resumable human questions.

The Linux supervisor observes BBP's existing exclusive moderator lock, starts a
separate process group with private task configuration, appends logs, and records
a diagnostic pid. Orch exit leaves the moderator running. Later polls restart
lost moderators; BBP itself revokes inherited turns. Pids are not the liveness
or ownership authority. Errors are surfaced, with details in the moderator log.
A moderator lifetime is bounded by `max_wall_secs`; a later Orch invocation may
start another lifetime for a still-active task.

Production always passes `--confine sandbox`. Windows and other unsupported
platforms fail closed. Windows parity requires a supported confined BBP runner
and process supervision; no unconfined fallback is provided. Provider login,
model quality, real merges, and Windows execution have not been exercised by
these deterministic tests.

## Validation

From the workspace root:

```sh
cargo test -p orch-core -p orch-dispatch -p orch-cli -p orch-store \
  -p orch-codex -p orch-claude -p orch-ollama -p rusty_orch \
  -p orch-bbp -p rusty_bbp -p rusty_bbp_host
cargo build -p rusty_orch -p rusty_bbp_host
BBP_TEST_BIN="$PWD/target/debug/bbp" \
ORCH_TEST_BIN="$PWD/target/debug/rusty_orch" \
cargo test -p orch-bbp --lib tests_e2e:: -- --ignored
cargo clippy -p orch-bbp -p orch-dispatch -p orch-store -p rusty_orch \
  -p rusty_bbp -p rusty_bbp_host --all-targets -- -D warnings
```

The explicit end-to-end tests exercise real binaries, lost replies after each
open step, persisted CLI resumes, moderator lock/restart, and a two-role MCP
workflow through runner evidence, independent review, approval and receipt.
Scripted workflow and running-workload lifecycle fixtures explicitly use
`--confine none` for their test workloads; production confinement is not configurable through Orch.

### Required Linux acceptance

The root CI workflow runs `orch-bbp-acceptance` on every event and includes it
in `required-gate`. It builds real `rusty_orch` and `bbp` binaries, then runs
`.github/scripts/orch_bbp_acceptance.py --output acceptance-results`. For local
Linux runs, first build those binaries and the `orch-bbp` library tests and
`rusty_bbp_host --test moderator_e2e`, then set `BBP_TEST_BIN` and `ORCH_TEST_BIN`
to their absolute paths. The collector requires exactly one executed test and
a completion marker for each of lifecycle, Rust/LLD linking, timeout, cancel and
kill, plus the separate `pause-adoption` regression. It attempts all six cases
and retains logs, JSON results and a Markdown summary on failure. The pause case
uses no workload: it verifies a durable failure pause across real Orch processes,
one task and one hosted-card charge, and adoption after an explicit owner reset.

The lifecycle uses the real Orch CLI, persisted state and production supervisor;
its workload must pass with `landlock+seccomp:no-endpoints` recorded in the BBP
report. It restarts the moderator at a plan gate, checks one task and one hosted
charge across process invocations, and requires approval followed by a separate
receipt before Orch finishes. Recovery cases require actual confined workloads
and descendants before interruption and verify cleanup before replacement.
Sandbox refusal fails these opt-in acceptance tests. The ordinary portable
suite still checks fail-closed refusal on unsupported kernels. Agent behavior is
scripted MCP; live provider credentials and real merges remain outside this test.

### Review corrections

Runner work is isolated under `work/<sha256(task id)>/run-N`; restart/adoption
uses the same task root. Invalid human syntax or a subject outside the displayed
gate now supersedes the answered question with an explanatory question. It does
not fail the card or retry immutable bad input. Transport failures remain
retryable with the original operation id. An expired request stays visible at
its escalation gate, with its body and message id, so the human can settle it
before separately confirming resume.

BBP moderation now invokes the existing runner as a separate process. The
moderator alone holds the runner's ownership pipe. Closing that pipe, including
on abrupt moderator death, cancels the active profile; the runner owns the wall
timer and reaps the workload. An exclusive runner lock remains held through
cleanup and report publication, preventing a replacement from preparing the
same checkout too early. Normal moderator exit waits for cleanup before
releasing its own lock. This hosted-card call budget is not a monetary cap;
the run guards merged in PR #763 independently enforce configured cost caps.

Manifest publication keeps the Unix directory fsync and avoids unsupported
Windows directory opens. Injected-seam tests are portable; production hosting
still rejects unsupported platforms. Native Windows checks run in repository CI.


A repeated no-progress launcher fault is a retryable host diagnostic, not a
permanent task failure. The BBP host replays its existing log to keep the pause
across Orch processes. Inspect `bbp mod-health`; after repairing the launcher,
use its explicit current-revision reset and poll the same goal/state again.
No new hosted-card charge, protocol event schema or `orch-core` change is needed.
For Rust/LLD workloads, generate `bbp profiles rust-lld --sysroot PATH` before
opening the task; see the host's provider-launcher guide for resource limits.
