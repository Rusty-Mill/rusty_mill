# ADR-0013: BBP task host: an `Implement` card opens one Blackboard Protocol task

- **Status:** Proposed
- **Date:** 2026-10-10

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
    /// Open the hosted task for `task`; returns its handle. One metered call.
    fn open(&mut self, task: &Task, board: &Board) -> Result<HostHandle, ClassifiedError>;
    /// Where the hosted task stands. Free: no call is counted.
    fn poll(&mut self, handle: &HostHandle) -> Result<Progress, ClassifiedError>;
    /// Pass a human answer through to the hosted task's human channel.
    fn relay(&mut self, handle: &HostHandle, answer: &Text) -> Result<(), ClassifiedError>;
}

pub struct HostHandle(pub Text);          // opaque to the dispatcher, e.g. "bbp:<dir>/<task>"

pub enum Progress {
    Working,                              // agents or the runner hold the task
    Gate { question: Text },              // a human must act; the text says how
    Closed { outputs: Vec<Output> },      // merged: receipt and candidate as Artifact entries
    Cancelled { reason: Text },           // cancelled or failed terminally
}
```

`Dispatcher::with_host(host)` is optional; without it an `Implement` card routes to `AgentRunner` as today, so nothing changes for existing goals.

### Dispatcher behaviour
- **Open.** A pending hosted card: route the agent as now, check both ceilings, `plan.start`, count one call, `host.open`. The handle is appended to the Board as an `Artifact` entry on the card (`refs = [handle]`, body "hosted on BBP"). The card stays `Running`. The handle is therefore persisted by the existing snapshot (ADR-0010) and rebuilt on resume by reading the card's live `Artifact` entries; no `TaskState` change and no `orch-core` change.
- **Poll.** Every `run` polls each hosted `Running` card before choosing the next ready card. `Working` leaves it; when nothing else is ready `run` returns the new `Outcome::Waiting(Vec<TaskId>)`, so a process can exit and resume later. `Gate` appends a `Question` entry (author: the card's agent) whose body is the gate text and calls `plan.block`, so the existing Blocked/Answer machinery and `--interactive` carry the human channel. `Closed` appends the outputs and `plan.complete`. `Cancelled` is a `ClassifiedError::Permanent`: the card fails with the reason.
- **Answer.** When a blocked hosted card's question gains an `Answer`, the dispatcher calls `host.relay(answer)` before resuming it, then polls. The answer body is a `bbp human` verb line (`approve-plan 7`, `approve-merge 12 3`, `reject planning too broad`, `answer 9 toolchain fixed`, `decision 9 reject not environmental`, `resume test`, `cancel ...`); the host validates it and passes the card revision it last read as `--rev`, so a stale answer is refused by BBP (`stale_rev`) and surfaces as a fresh `Gate` with the current card.
- **Metering.** One orch call per hosted card, at open. BBP's own per-field budgets meter the turns inside; `Budget::max_calls` counts cards opened, not turns. Stated in the goal file documentation.

### `orch-bbp` adapter
A new crate `crates/apps/rusty_orch/crates/orch-bbp` implementing `TaskHost` over the `bbp` binary through `orch_cli::CommandRunner` (fixed argv, no shell, deadline, env scrub), depending on `orch-dispatch`, `orch-cli` and `rusty_bbp` (for `Card`, `State`, `Role` and the id types that decode `bbp card`). It never links `rusty_bbp_host`.

- `open`: writes the brief (card instruction, acceptance items, the goal's DONE WHEN and constraints, the card's refs) and the profile set (repository test command, read roots, environment, limits from the goal file; default `cargo test`), runs `bbp open --dir <state>/bbp --task <goal>-<card> --repo <repo> --brief F --profiles P`, then `bbp assign` for the four BBP roles, then starts `bbp mod ... --agents <launchers> --confine sandbox` in its own process group, detached, logging under the task directory. The handle is `bbp:<dir>/<task>`.
- `poll`: `bbp card` (JSON) decoded as `rusty_bbp::Card`. `closed` → `Closed` with the merge receipt and candidate as `Artifact` outputs; `cancelled` → `Cancelled`; `plan_gate`, `merge_gate`, `escalated`, or a pending non-gate request → `Gate` with a text naming the exact verbs and ids the human may use; anything else → `Working`. If the moderator lock is free (a previous orch process exited), `poll` restarts `bbp mod` first; a start refused by the lock is not an error.
- `relay`: `bbp human --dir --task --rev <card.rev> <verb line>`.
- Role to vendor: the card's routed `Agent` serves Planner, Coder and Tester; the Reviewer is `Routing::reviewer(author)`, so the vendor that reviews is never the one that wrote (ADR-0005 carried into BBP, which already isolates the Reviewer). Launchers come from a template per `Agent` (Claude: the proving-run `agents.json` launchers; Codex: `codex exec`; Local: `ollama`) with the BBP placeholders `{mcp_config}`, `{task}`, `{turn}`.
- Failure classes: `bbp` missing or a vendor CLI not logged in → `Unavailable` (card stays resumable, no call charged); a non-zero `bbp open`/`assign` with a stated refusal → `Permanent`; a transport failure around `card` → `Transient`.

### Goal file and command line
`repo` (already present for Codex and Claude) plus optional `test` (profile command), `read_roots`, `env`, `limits`, and `bbp` (binary path, env `ORCH_BBP`). `rusty_orch run --bbp <path>` enables the host; `Outcome::Waiting` prints the hosted cards and their handles and exits 0 so a cron or a human can run again. `--interactive` prompts with the gate text and relays the typed verb line.

## Consequences
- The Board shrinks for hosted cards to a pointer, the gate questions and answers, and the closing artifacts. Discussion, verdicts, runs and diffs live in the BBP store and are read there (the GUI drill-down reads BBP directly through the handle; orch does not mediate turn history). Full Board retirement waits for a BBP task shape for non-code cards; out of scope here.
- Additive source changes in-family: `Outcome::Waiting` (exhaustive matches), `Dispatcher::with_host`, the `TaskHost` trait and types, `AgentsConfig`/`Args` fields. No `orch-core` change.
- `FakeHost` in `orch-dispatch` for the dispatcher tests (open, working, gate, answer relay, closed, cancelled, resume from a persisted handle). `orch-bbp` gets a contract test pinning every argv and an ignored real-binary test that runs a scripted task under `--confine none` with launchers that post through `bbp mcp`, as `rusty_bbp_host`'s own e2e does.
- Phasing: (a) port, fake, `Waiting`; (b) `orch-bbp` over the CLI with the e2e; (c) goal file, flags, interactive relay; (d) documentation of the slimmer Board. Each is one PR.

## Alternatives rejected
- **BBP task as an `AgentRunner`.** One call is not one task; the port forbids internal loops, and a card blocked for hours inside `run` cannot be resumed by a new process.
- **Dispatcher as BBP principal proxying the CLI adapters.** That is the relay loop BBP was designed to replace: the dispatcher would read the blackboard for the agent and paste it into a prompt, losing the pull-based, token-fenced reads.
- **Linking `rusty_bbp_host`.** Refused by the layer check (`cross-family apps dependency`), and the CLI is the surface BBP keeps stable; moving the host's library parts under `libs/` can be revisited if the JSON card proves too thin a contract.
