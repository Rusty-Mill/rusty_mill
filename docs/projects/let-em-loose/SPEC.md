# Let Em Loose: spec

Status: concept, 2026-11-02. Concept: [README.md](README.md). Decisions: [DECISIONS.md](DECISIONS.md). Pre-answered questions: [QUESTIONS.md](QUESTIONS.md).

## What it is for

Let several models and agents build software together from a human's intent, without the human being the bottleneck.

The factory pipeline (planner → coder → tester → reviewer) is replaced by **critic lenses** working on one shared board of **claims**. Agents post evidence; the board computes confidence; work converges when no lens can find a problem worth the cost. The human writes intent and decides only exceptions: irreversible actions, taste, deadlocks and budget overruns.

## Users

| Who | Does |
| --- | --- |
| Nano (owner) | Writes and revises intent. Answers the escalation inbox. Signs off the golden set. |
| Lenses | Stateless model calls that answer one targeted question and post evidence or claims. |
| Builders | Tool-using agents (Claude Code, Codex) that change code in a sandbox and post results as evidence. |

## Scope

### v0: the thin slice

The smallest thing that runs end to end. Its first real job is the dedup experiment.

1. **Claims model in BBP** (#593). Claim and Evidence as first-class board objects. Deterministic confidence update, dependencies, accept/drop/escalate thresholds, settling detection, stakes-scaled budgets. Built on event-driven, concurrent evidence posting.
2. **Lens runner in orch** (#594). One port, one adapter per provider (Claude, OpenAI/Codex, Gemini, local llama-swap). Per-question context assembly. Two or three critic lenses plus a builder. Blind first pass, evidence-required agreement and look-harder rules enforced as checks.
3. **Decision inbox** (#595). Escalations only. Approve, Reject, Refine intent.
4. **Intent format v0.** Goal, targets, always, never, examples, taste, reversibility. Versioned.
5. **remind-me active-project filtering** (#592), audit first.
6. **The dedup experiment.** Intent for a 20 TB duplicate-file manager with a GUI, run in a fresh empty repo, compared against `rusty_fclone`.

### v1: after the slice works

- Value-driven scheduling: value scoring, the board asks, fork and collapse (#596), as a shadow-mode experiment.
- Ground truth: holdout tests, golden set, calibration tracking (#597).
- Existing code, sandbox and data boundaries, intent versioning, runaway cost (#598).
- Cross-project lessons, difficulty-weighted track records, post-ship maintenance, decision replay (#599).
- The full UI: intent panel, zoomable claims map, inbox, spend meter.

## Non-goals

- **Agents talking to each other.** All exchange goes through the board. No agent-to-agent calls, no prose debate.
- **A pipeline with stages.** Planning, building and testing are concurrent activities, not phases.
- **Vendor lock-in.** No model or vendor logic in orch core.
- **Roles in BBP.** BBP stays a pure protocol.
- **A fourth executor.** The lens runner extends or replaces one of the three that exist.
- **A new platform repo.** The build lives in rusty_mill.
- **Autonomous irreversible actions.** They always reach the inbox.
- **Agents merging PRs.** Repository rule: agents open PRs and stop.
- **Brownfield in v0.** v0 targets greenfield work from intent; existing code is #598.
- **UI before the slice.** The CLI and the inbox come first.

## Constraints

- **Repository rules.** Every change through a PR with a change fragment. Review by `dottiusmaximus`. Merge commit through the merge queue. No force pushes.
- **Design first.** Every card posts a design for Nano's approval before implementation.
- **Ask first:** schema migrations, BBP schema version bumps, public API changes, dependency or toolchain bumps, deletions, adding or removing an executor.
- **Rust.** `Result` and `?` in library code; no `unwrap()` or `expect()` outside tests. Ports and adapters, domain logic free of I/O. No abstraction before two real call sites. Tests for all non-trivial logic.
- **Determinism.** The confidence update, thresholds, settling and scoring are pure functions over the evidence log, reproducible in trace tests.
- **Serialized control plane.** State transitions, budgets and human gates are serialized. Evidence posting is concurrent.
- **Sovereignty.** Each adapter declares what data it may receive. Sensitive claims route to the local model.
- **Auditability.** Every confidence change is traceable to the evidence that caused it.
- **Sandboxing.** Nothing touches real user data before the shadow run, and shadow runs are read-only.
- **Handoffs.** Commands handed to Nano are PowerShell.

## Components and ownership

| Component | Location | Owns |
| --- | --- | --- |
| BBP | `crates/libs/protocol/rusty_bbp` | Claim and evidence semantics, confidence rule, thresholds, settling, budgets, escalation, event log |
| orch | `rusty_orch` | Lens definitions, model adapters, context assembly, which lens is asked, tiering, enforcement of the anti-groupthink rules |
| Inbox | Mill Board view over rusty_tick (to confirm in #595) | Escalation cards and Nano's three actions |
| remind-me | its own repo after the planned split | Cross-project lessons, decisions, active-project filtering |
| Factory repo | `baileyrd/Factory` | Process and config only: intent template, lens definitions, model bindings |
| Expert-Agents | `baileyrd/Expert-Agents` | Source prompts for lenses |
| JARVIS | Hermes harness | The face: priorities and talking to Nano. Not the engine. |

## Success criteria for v0

1. The dedup experiment runs from intent alone to a working prototype, with no access to `rusty_fclone`.
2. Nano's time is the intent interview plus a handful of inbox decisions, roughly an hour in total.
3. The result is compared against `rusty_fclone` on scan throughput, crash-resume loss, correctness against the golden set, GUI usability as judged by Nano, and tokens spent.
4. Every accepted claim can be traced to its evidence.
5. No irreversible action happened without an inbox approval.

## Glossary

- **Intent:** what must be true, must never happen, examples and taste. Written by Nano and versioned.
- **Claim:** a small proposition with a confidence, evidence and dependencies.
- **Evidence:** a direction (support or counter), strength, kind (executable, measurement, citation, opinion), source and author, attached to a claim.
- **Lens:** a critic perspective (adversary, operator, research, security and so on) bound to a model by config.
- **Settled:** recent evidence no longer moves a claim's confidence.
- **Escalation:** a claim sent to Nano's inbox.
- **Rebel candidate:** a candidate that deliberately departs from prior art.
- **Shadow mode:** run alongside without acting; record what would have happened.
