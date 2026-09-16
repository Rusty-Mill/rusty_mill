# Glossary — harness vocabulary

> **Authoritative source** for concise definitions of the Rusty Keys / *AI Harness Engineering* vocabulary. Each entry is a short gloss plus a link to the doc that owns the full treatment; this file defines terms, it does not re-specify them. [`ARCHITECTURE.md`](../ARCHITECTURE.md) §13 points here.

Terms are grouped by theme. Authoritative docs: [`ARCHITECTURE.md`](../ARCHITECTURE.md) · [`architecture/data-model.md`](../architecture/data-model.md) · [`dev/eval-plan.md`](../dev/eval-plan.md) · the PRDs ([`prd/`](../prd/)) · the ADRs ([`adr/`](../adr/README.md)).

---

## Conceptual foundation

**Kernel / harness split** — The model's agent loop is the *kernel* (Decide + Act, the aisdk `LanguageModelRequest` loop); the application built around it is the *harness* (constrain, feed, observe, compose). The kernel is deliberately thin and knows nothing about memory, policy, or verification. → [ARCHITECTURE.md §2](../ARCHITECTURE.md), [PRD 01](../prd/01-kernel.md), [ADR-0005](../adr/0005-harness-decomposed-into-four-verbs.md).

**The four verbs** — The harness decomposition: **constrain** (vet every tool call before dispatch), **feed** (tools, context, memory, Task State), **observe** (episode trace, interventions, entropy), **compose** (verification, attribution, evidence). Each verb is one crate; each maps onto an OODA phase. → [ARCHITECTURE.md §2](../ARCHITECTURE.md), [ADR-0005](../adr/0005-harness-decomposed-into-four-verbs.md).

**OODA loop** — Observe–Orient–Decide–Act. The four verbs map onto it: constrain gates, feed = Observe+Orient (memory capture + recall), kernel = Decide+Act, compose verifies. Memory is specifically the Observe+Orient half. → [ARCHITECTURE.md §2](../ARCHITECTURE.md), [ADR-0008](../adr/0008-memory-is-observe-orient-half-of-ooda.md).

**Ashby's Law of Requisite Variety** — A regulator must have at least as much variety as the system it governs. It is the justification for the harness: an unconstrained model has effectively infinite possibilities, and the harness supplies the necessary variety-reduction via state tracking, tool permissions, and deterministic checks. → [ARCHITECTURE.md §2](../ARCHITECTURE.md), [PRD 00](../prd/00-overview.md).

**`C_system = F(C_model, C_harness, C_environment, T)`** — The paper's central equation: system capability is an emergent property of model, harness, environment, and task together — not of the model alone. Verbatim-faithful to the paper. → [ARCHITECTURE.md §2](../ARCHITECTURE.md).

---

## Maturity & measurement

**H0–H3** — The maturity ladder, intended as a controlled-visibility ablation. **H0**: task + repo files, no tool registry (ablation floor — selectable or eval-only, ADR-0028). **H1**: tool registry + tool-use protocol. **H2**: project memory, Task State, context selection. **H3**: deterministic checks, attribution, verification protocol. → [ARCHITECTURE.md §3](../ARCHITECTURE.md), [dev/eval-plan.md §4](../dev/eval-plan.md), [ADR-0028](../adr/0028-h0-selectable-harness-level-or-eval-only.md).

**M-HIR** (Missing-Harness Human Intervention Rate) — `count(interventions where avoidability != benign) / count(turns)`. Only non-`benign` interventions enter the numerator (that is the *M* — missing-harness — vs. raw HIR). A falling rate signals harness improvement. RK uses **turns** as the denominator, a deliberate divergence from the paper's **episodes**. → [PRD 04](../prd/04-observe.md), [dev/eval-plan.md §2](../dev/eval-plan.md), [ADR-0019](../adr/0019-intervention-model-maps-to-avoidability-harness-gap-burden.md).

**Episode package** — The paper's central output artifact: an auditable per-turn JSON record carrying **all eight traces** (`action_trace`, `tool_trace`, `context_trace`, `verification_trace`, `attribution_log`, `reproduction_log`, `verification_report`, `intervention_log`, plus entropy + outcome). Written at H3 to `episodes/<turn_id>.json`. RK's episode = one `send()` turn; `episode_id` groups a task's turns. → [data-model §5](../architecture/data-model.md), [PRD 05](../prd/05-compose.md), [ADR-0018](../adr/0018-episode-equals-turn-with-episode-id-grouping.md).

---

## Outcome taxonomy (`EpisodeOutcome`, 5 labels)

The five-label classification of every H3 turn, snake_case on the wire. → [PRD 05](../prd/05-compose.md), [data-model §5/§7](../architecture/data-model.md).

- **`AutonomousVerifiedSuccess`** — All checks pass, verification report produced, **no interventions**. Barred if the judge was unavailable.
- **`AssistedVerifiedSuccess`** — Checks pass, but human interventions were recorded during the turn.
- **`UnverifiedSuccess`** — Task appears done but the agent produced no verification report.
- **`Failed`** — Required checks fail, or no usable reply was produced.
- **`UnsafeInvalid`** — Tests weakened, unrelated destructive edits, or task bypassed. Triggered by an entropy finding with `severity ≥ 2` on `TestWeakening`/`BoundaryViolation`.

---

## Failure attribution (`FailureType`, 8 variants)

The fixed 8-member failure taxonomy adopted from the paper (replacing free strings), snake_case on the wire. Makes attribution aggregatable across episodes. → [data-model §5](../architecture/data-model.md), [PRD 05](../prd/05-compose.md), [ADR-0021](../adr/0021-fixed-failuretype-taxonomy.md).

- **`f_context`** — wrong/insufficient context fed to the model.
- **`f_tool`** — a tool failed or behaved incorrectly.
- **`f_feedback`** — the agent ignored or misread feedback from a prior step.
- **`f_verify`** — verification was skipped, wrong, or insufficient.
- **`f_recovery`** — the agent failed to recover from a detected error.
- **`f_entropy`** — failure rooted in introduced maintenance burden.
- **`f_model`** — a reasoning error attributable to the model itself.
- **`f_unknown`** — cause not determinable (a high share signals weak attribution).

---

## Entropy audit (`EntropyCategory`, RK's 6 ↔ paper's 7)

Maintenance burden the agent introduces without breaking the immediate task. Each finding carries a **0–3 severity** (0 = informational, 3 = significant); `delta = -Σ severity`. Entropy is informational, never a gate. → [PRD 04](../prd/04-observe.md), [data-model §4.4](../architecture/data-model.md), [ADR-0020](../adr/0020-entropy-categories-six-reconciled-to-seven.md).

RK's **6 categories**: `Residue` (debug/temp files, dead code), `TestWeakening` (removed assertion, `#[ignore]`), `StaleDocs` (doc contradicted by a code change), `DependencyChurn` (dep added-then-removed or unused), `BoundaryViolation` (write outside `TaskState.scope`), `TaskContradiction` (comment contradicts the task goal).

**RK 6 → paper 7 reconciliation** (ADR-0020): the paper has 7 (*code, documentation, dependency, test, file-residue, architecture, workflow*). RK's `Residue` covers the paper's **code + file-residue** (two paper categories → one RK), and RK's `TaskContradiction` is the paper's **workflow** (rename); the remaining four are 1:1. This map lets RK entropy be compared to paper figures. → [ARCHITECTURE.md §12](../ARCHITECTURE.md), [ADR-0020](../adr/0020-entropy-categories-six-reconciled-to-seven.md).

---

## Interventions (`InterventionKind`, 7 RK kinds)

An **intervention** is any human action that compensates for a missing or insufficient harness capability; interventions drive M-HIR. The paper classifies by `avoidability` / `harness_gap` / `burden`; RK adds **7 UI-observable kinds** on top of those three paper-aligned fields (the kinds are an RK invention, ADR-0019). → [PRD 04](../prd/04-observe.md), [data-model §4.2](../architecture/data-model.md), [ADR-0019](../adr/0019-intervention-model-maps-to-avoidability-harness-gap-burden.md).

The 7 kinds: `task_override`, `manual_reflect`, `manual_groom`, `manual_verify`, `unverified_followup`, `tool_block`, `direct_edit`. Each record also carries `avoidability` (`avoidable | unavoidable | benign`), `harness_gap`, and `burden` (0–3); only non-`benign` records count toward M-HIR.

---

## Memory & feed

**Task State** — The working-memory tier: a single current goal + `success_criteria` + `scope`, persisted to `task.json`, carried across turns. Injected into the system prompt for drift prevention and anchors recall. Distinct from the in-session `task_create` background-operation registry. → [PRD 03](../prd/03-feed.md), [data-model §8](../architecture/data-model.md).

**Consolidation** — Distillation of the short-term observation stream into the long-term memory graph, at three tempos: idle (micro), sleep (session end), explicit (user command). An async aisdk call emits structured `Memory` records; UNVERIFIED outcomes become high-importance `skill` memories. → [PRD 03](../prd/03-feed.md), [ADR-0009](../adr/0009-tiered-consolidation-idle-sleep-explicit.md).

**Recall** — The Orient step: each turn assembles relevant long-term memories (blending relevance + recency + importance; semantic when an embed model is set, FTS5 lexical otherwise) into the context block fed to the kernel. → [PRD 03](../prd/03-feed.md), [data-model §3](../architecture/data-model.md).

**Evidence journal** — The append-only JSONL record (`evidence.jsonl`) of every turn's verification package, every consolidation changelog, and every compaction event. Completion is an auditable record, not a claim; `count_turns()` over it supplies the M-HIR denominator. → [PRD 05](../prd/05-compose.md), [data-model §4.1](../architecture/data-model.md), [ADR-0015](../adr/0015-evidence-journal-append-only-jsonl.md).

**`ToolOutcome`** — The single structured tool-result type carrying tool status (`ok` / `error` / `blocked`) **structurally**, with one formatter/parser to render it to and from the model-facing string. Replaces fragile magic-prefix (`ERROR …`/`BLOCKED …`) inference; observe consumes the status directly. → [data-model](../architecture/data-model.md), [dev/error-handling.md](../dev/error-handling.md), [ADR-0022](../adr/0022-structured-tooloutcome-tool-result-contract.md).

---

## Architecture seams

**`SessionFactory`** — A trait defined in the low `feed` crate and implemented by `app`, letting the `agent` subagent tool construct a child `Session` without creating a `feed → app` dependency cycle. The seam that keeps the crate DAG acyclic. → [ARCHITECTURE.md §5](../ARCHITECTURE.md), [PRD 06](../prd/06-app.md), [ADR-0017](../adr/0017-subagent-spawning-via-sessionfactory-trait.md).
