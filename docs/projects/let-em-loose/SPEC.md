# Let Em Loose: spec

Status: designs for #592–#597 approved 2026-11-02; implementation starts with the #593 engine. BBP rules: the claims addendum (PR #1386). Concept: [README.md](README.md). Decisions: [DECISIONS.md](DECISIONS.md). Pre-answered questions: [QUESTIONS.md](QUESTIONS.md).

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

1. **Claims model in BBP** (#593). Claims as a per-task mode beside the stage lifecycle. Whole-number scores, two-author acceptance, the failing-test block, supersession, dependency collapse, action marks, stakes-scaled budgets. Evidence posts need no turn. Same log, same outcome.
2. **Lens runner in orch** (#594). A config-driven provider registry with Claude, Codex and local (llama-swap through a new OpenAI-compatible adapter). Three blind Architect proposals per question, then Research, Adversary and Operator. Fresh context per question. The groupthink rules enforced as code checks.
3. **Decision inbox** (#595). A read view over BBP in Mill Board. Approve, Reject, Choose, Refine intent.
4. **Ground truth** (#597). Holdout suites required for acceptance, protected test paths, golden sets signed off by Nano, calibration recorded.
5. **Intent format v0.** Goal, targets, always, never, examples, taste, reversibility. Versioned.
6. **remind-me project parameter** (#592), sent explicitly by every client.
7. **The dedup experiment** (#600). Intent for a 20 TB duplicate-file manager with a GUI, run in a fresh empty repo, compared against `rusty_fclone`.

### v1: after the slice works

- Value-driven scheduling (#596): shadow mode, then paired runs on five Phase B intents against a pre-registered rule.
- Calibration that changes weights (recorded but informational in v0).
- Existing code, sandbox and data boundaries, intent versioning, runaway cost (#598).
- Cross-project lessons, difficulty-weighted track records, post-ship maintenance, decision replay (#599).
- The full UI: intent panel, zoomable claims map, inbox, spend meter.

## Non-goals

- **Agents talking to each other.** All exchange goes through the board. No agent-to-agent calls, no prose debate.
- **A pipeline with stages.** Planning, building and testing are concurrent activities, not phases.
- **Vendor lock-in.** No model or vendor logic in orch core.
- **Replacing BBP's stage lifecycle in v0.** Claims are an opt-in mode beside it.
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
| orch | `rusty_orch` (lens runner beside the dispatcher; `orch-openai` adapter) | Lens definitions, provider registry, context assembly, which lens is asked, tiering, the groupthink rules, value scheduling, calibration records |
| Inbox | Mill Board, reading BBP through the host's `bbp` command | Escalation cards and Nano's four actions; nothing stored in rusty_tick |
| remind-me | its own repo after the planned split | Cross-project lessons, decisions, active-project filtering |
| Factory repo | `baileyrd/Factory` | Process and config only: intent template, lens definitions, model bindings, experiment intents, holdout suites, `golden/` sets |
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
- **Candidate:** a built version (base plus diffs) tied to a claim. Storing one starts its sandboxed run.
- **Epoch:** one attempt at settling a claim. A reopen starts a new epoch, which must re-earn its settlement.
- **Profile:** a named test set, frozen by the host when a task opens.
- **Holdout suite:** tests written from intent alone that only the holdout lens can run.
- **Action mark:** the host's record that it is about to act on a claim, checked against the epoch.
- **Rebel candidate:** a candidate that deliberately departs from prior art.
- **Shadow mode:** run alongside without acting; record what would have happened.
