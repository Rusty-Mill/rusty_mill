# ADR-0013: BBP task host: an `Implement` card opens one Blackboard Protocol task

- **Status:** Proposed
- **Date:** 2026-10-10 (amended the same day after design review: crash-recoverable open, persisted gate fence, `bbp show`, the `approved` gate, decoded replies, the supervisor seam)

## Context
The BBP decision record (`crates/libs/protocol/rusty_bbp/docs/review/orch-core-relationship.md`, 2026-10-08) fixed the layering: rusty_orch sits above BBP, a code-producing card becomes one BBP task, and the orch Board retires for such cards once adapters post to BBP instead of returning `Output`. It left one question for BBP stage 3: do the CLI adapters become BBP agents under a BBP MCP server, or does the dispatcher become a BBP principal that proxies them? Stage 3 has landed (`rusty_bbp_host`: per-turn MCP server, human channel, sandboxed runner, moderator, PRs #553 to #601), the first proving run has executed, and its environment faults are fixed. This ADR answers that question and is the design for roadmap Phase 1.

Three facts shape it. The dispatcher's agent port is one metered model call per invocation (`AgentRunner`, "must not loop internally"); a BBP task is many turns, two human gates and a runner, lasting hours. The workspace layer check refuses a cross-family dependency between `apps/` crates, so `rusty_orch` cannot link `rusty_bbp_host`; it may link the libs crate `rusty_bbp`. And the `bbp` binary is already the fenced surface BBP wants driven: `open`, `assign`, `card` (JSON), `human`, `mod`.

## Decision
### Boundary
A `Role::Implement` card whose goal names a repository is hosted on BBP. Research, Design, Triage and Review cards keep the orch Board and the existing adapters unchanged. The adapters do not become BBP agents and the dispatcher does not proxy them: the BBP moderator launches the vendor CLIs itself, as it does in the proving run, and rusty_orch acts only as the BBP **human** principal for the task.

### Port
A second port in `orch-dispatch`, beside `AgentRunner`:

```rust
pub trait TaskHost {
    /// Open the hosted task for `task`, or adopt one already open for it.
    /// Idempotent: safe to call again after a crash at any point.
    fn open(&mut self, task: &Task, board: &Board) -> Result<HostHandle, ClassifiedError>;
    /// Where the hosted task stands. Free: no call is counted.
    fn poll(&mut self, handle: &HostHandle) -> Result<Progress, ClassifiedError>;
    /// Pass a human answer through. `gate` is the Board `Question` the
    /// answer replies to; its refs carry the fence the answer is held to.
    fn relay(&mut self, handle: &HostHandle, gate: &Entry, answer: &Text)
        -> Result<Relayed, ClassifiedError>;
}

pub struct HostHandle(pub Text);          // opaque to the dispatcher: "bbp:<dir>/<task>"

pub enum Progress {
    Working,                              // agents or the runner hold the task
    Gate(GateContext),                    // a human must act
    Closed { outputs: Vec<Output> },      // merge receipt recorded: candidate and revision
    Cancelled { reason: Text },
}

/// Everything the human was shown, so the answer can be held to it.
pub struct GateContext {
    pub prompt: Text,                     // what to do and the exact verbs allowed
    pub rev: u64,                         // card revision shown
    pub ids: Vec<Text>,                   // "art:12", "run:3", "msg:9": the only ids an answer may name
}

pub enum Relayed { Applied, Stale(GateContext) }  // Stale: the fence moved; here is the new gate
```

`Dispatcher::with_host(host)` is optional; without it an `Implement` card routes to `AgentRunner` as today, so nothing changes for existing goals.

### Dispatcher behaviour
- **Open, crash-recoverable.** A pending hosted card: route the agent as now, check both ceilings, `plan.start`, count one call, `host.open`. The handle is appended to the Board as an `Artifact` entry on the card (`refs = [handle]`, body "hosted on BBP") and the card stays `Running`. rusty_orch checkpoints only after `Dispatcher::run` returns (`run.rs`), so the count and the handle reach the snapshot together or not at all: a crash before the checkpoint leaves the card `Pending` and the ledger uncharged, the retry counts once more and `open` adopts the task BBP already holds. Exactly one persisted charge per hosted task; no `TaskState` change and no `orch-core` change. On resume the handle is rebuilt from the card's live `Artifact` entries.
- **Poll.** Every `run` polls each hosted `Running` card before choosing the next ready card. `Working` leaves it; when nothing else is ready `run` returns the new `Outcome::Waiting(Vec<TaskId>)`, so a process can exit and resume later. `Gate` appends a `Question` entry (author: the card's agent) whose body is the prompt and whose refs are the handle, `bbp:rev:<rev>` and the context ids, then `plan.block`, so the existing Blocked/Answer machinery and `--interactive` carry the human channel and the fence is persisted with the question, not held in adapter memory. `Closed` appends the outputs and `plan.complete`. `Cancelled` is a `ClassifiedError::Permanent`: the card fails with the reason.
- **Answer.** When a blocked hosted card's question gains an `Answer`, the dispatcher calls `host.relay(handle, question, answer)` before resuming it. The answer body is a `bbp human` verb line. `Applied` resumes the card and polls; `Stale` appends a new `Question` superseding the old one with the current gate, and the card stays blocked. A later poll never refreshes the fence of an answer already given.
- **Metering.** One orch call per hosted card, at open. BBP's own per-field budgets meter the turns inside; `Budget::max_calls` counts cards opened, not turns. Stated in the goal file documentation.

### `orch-bbp` adapter
A new crate `crates/apps/rusty_orch/crates/orch-bbp` implementing `TaskHost` over the `bbp` binary, depending on `orch-dispatch`, `orch-cli` and `rusty_bbp` (for `Card`, `Response`, `Code`, `State`, `Role` and the id types). It never links `rusty_bbp_host`.

- **Two process seams.** Finite commands (`open`, `assign`, `card`, `show`, `human`) run through `orch_cli::CommandRunner` (fixed argv, no shell, deadline, env scrub). The moderator cannot: `CommandRunner` waits for the child and kills its group at the deadline. A `Supervisor` seam in `orch-bbp` owns long-lived children: `ensure_moderator(handle) -> Running | Started | Refused`, which spawns `bbp mod ... --agents <launchers> --confine sandbox` as the leader of its own process group with stdin null and stdout/stderr appended to `<dir>/mod/<task>.log`, records the pid at `<dir>/mod/<task>.pid`, and does not wait. The moderator's own lock (`<dir>/mod/<task>.lock`) makes a second start `Refused`, which is not an error. rusty_orch never kills the moderator on exit; it ends when the task closes or is cancelled, or at its own `--max-wall-secs`. `StdSupervisor` is real, `FakeSupervisor` records launches.
- **Every `bbp` reply is decoded, not inferred from the exit code.** The binary prints a `rusty_bbp::Response` as JSON and exits 0 for a protocol refusal (`Rejected { code, detail }`), non-zero only for an argument or I/O failure. The adapter maps: binary missing → `Unavailable`; non-zero exit → `Transient` with stderr excerpt; `Ok`/`Posted`/`Stored` → success; `Rejected`: `AlreadyOpen` → adopt, `StaleRev` → `Relayed::Stale` after a fresh `show`, `WrongState` or `Terminal` → re-poll, anything else → `Permanent` naming the code. Contract tests cover the exit-zero `AlreadyOpen` and `stale_rev` replies.
- **`open`, idempotent.** The BBP task id is `<goal>-<card>`, so a retry finds the same task. Sequence: `show` (exists → adopt and continue); write the brief (card instruction, acceptance items, the goal's DONE WHEN and constraints, the card's refs) and the profile set (repository test command, read roots, environment, limits from the goal file; default `cargo test`); `bbp open` (`AlreadyOpen` → adopt); `bbp assign` for the four roles (the engine replaces an existing assignment, so repeating is safe); `ensure_moderator`. Every step is safe to repeat, so a crash after any of them is recovered by calling `open` again. Tests interrupt after each external step and before and after the snapshot.
- **`poll` reads `bbp show`, not `bbp card`.** The `Card` lacks what the gates need: the proposed spec is `null` at the first `plan_gate` (`approved_spec` is set by the approval), the pending request's body and requester are not on it, and the merge receipt and the cancellation reason live only in the event log. `bbp show` is a new host command (prerequisite PR, `rusty_bbp_host`) printing one JSON document: the `Card`, `gate: { kind, proposal: <art id of the gate request's spec>, request: { msg, requester, body } }` when a gate or request is pending, and `closing: { receipt: { candidate, revision } | cancelled: { reason } | escalated: { reason } }` folded from the events. The `Card` struct is unchanged, which also answers the open question of exposing the gate proposal on it: not needed.
- **Gates, including `approved`.** `plan_gate` → prompt "approve-plan <spec> | reject planning <reason>", ids `art:<spec>`; `merge_gate` → "approve-merge <cand> <run> | reject build <reason>", ids `art:<cand>`, `run:<run>`; `approved` → "candidate <cand> is approved on run <run>: merge it outside BBP, then `receipt <cand> <revision>`", ids `art:<cand>` (approval alone does not close the task; only the receipt does, so `approved` is a gate, never `Working`); `escalated` → "resume <state> | cancel <reason>" with the escalation reason; a pending non-gate request → "answer <msg> <text> | decision <msg> accept|reject <note>" with the request body, ids `msg:<msg>`. `closed` → `Closed` with two `Artifact` outputs, the candidate and the receipt revision; `cancelled` → `Cancelled` with the reason; everything else → `Working`. `poll` also calls `ensure_moderator` first, so a moderator lost with a previous orch process is restarted.
- **`relay` holds the answer to the question.** Revision-fenced verbs (`reject`, `rerun`, `resume`, `cancel`) are sent with `--rev <rev from the question's refs>`, never the adapter's latest read. Subject-guarded verbs (`approve-plan ART`, `approve-merge ART RUN`, `receipt ART REV`, `decision MSG`, `answer MSG`) must name only ids from the question's refs; an answer naming any other id is refused by the adapter before `bbp` sees it. A `StaleRev` or `WrongState` reply becomes `Relayed::Stale` with the gate as `show` reports it now.
- **Role to vendor.** The card's routed `Agent` serves Planner, Coder and Tester; the Reviewer is `Routing::reviewer(author)`, so the vendor that reviews is never the one that wrote (ADR-0005 carried into BBP, which already isolates the Reviewer). Launchers come from a template per `Agent` (Claude: the proving-run `agents.json` launchers; Codex: `codex exec`; Local: `ollama`) with the BBP placeholders `{mcp_config}`, `{task}`, `{turn}`.
- **Failure classes.** `bbp` missing or a vendor CLI not logged in → `Unavailable` (card stays resumable, no call charged); a `Rejected` other than the handled codes, or a `cancelled` task → `Permanent`; a transport failure around `show` → `Transient`.

### Goal file and command line
`repo` (already present for Codex and Claude) plus optional `test` (profile command), `read_roots`, `env`, `limits`, and `bbp` (binary path, env `ORCH_BBP`). `rusty_orch run --bbp <path>` enables the host; `Outcome::Waiting` prints the hosted cards and their handles and exits 0 so a cron or a human can run again. `--interactive` prompts with the gate prompt and relays the typed verb line.

## Consequences
- The Board shrinks for hosted cards to a pointer, the gate questions and answers, and the closing artifacts. Discussion, verdicts, runs and diffs live in the BBP store and are read there (the GUI drill-down reads BBP directly through the handle; orch does not mediate turn history). Full Board retirement waits for a BBP task shape for non-code cards; out of scope here.
- Additive source changes in-family: `Outcome::Waiting` (exhaustive matches), `Dispatcher::with_host`, the `TaskHost` trait and its types, `AgentsConfig`/`Args` fields. No `orch-core` change. One `rusty_bbp_host` addition: `bbp show`.
- `FakeHost` in `orch-dispatch` for the dispatcher tests: open, adopt after a simulated crash before the checkpoint (one persisted charge), working, each gate kind including `approved`, answer relay applied and stale, closed, cancelled, resume from a persisted handle. `orch-bbp` gets contract tests pinning every argv and the decoding of exit-zero `Rejected` replies (`AlreadyOpen`, `StaleRev`), interruption tests after each `open` step, and an ignored real-binary test that runs a scripted task under `--confine none` with launchers that post through `bbp mcp`, through approval, receipt and `Closed`, as `rusty_bbp_host`'s own e2e does.
- Phasing, one PR each: (0) `rusty_bbp_host`: `bbp show`; (a) port, fakes, `Waiting`; (b) `orch-bbp` with the `Supervisor` seam and the e2e; (c) goal file, flags, interactive relay; (d) documentation of the slimmer Board.

## Alternatives rejected
- **BBP task as an `AgentRunner`.** One call is not one task; the port forbids internal loops, and a card blocked for hours inside `run` cannot be resumed by a new process.
- **Dispatcher as BBP principal proxying the CLI adapters.** That is the relay loop BBP was designed to replace: the dispatcher would read the blackboard for the agent and paste it into a prompt, losing the pull-based, token-fenced reads.
- **Linking `rusty_bbp_host`.** Refused by the layer check (`cross-family apps dependency`), and the CLI is the surface BBP keeps stable; moving the host's library parts under `libs/` can be revisited if the JSON card proves too thin a contract.
