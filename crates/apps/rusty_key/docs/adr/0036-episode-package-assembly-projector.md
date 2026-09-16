# ADR-0036: Episode-package assembly projector at `compose`-time

- Status: Accepted
- Date: 2026-05-27
- Tags: compose, observe, data-model, faithfulness

## Context

The `EpisodePackage` declares eight typed traces and the `EvidenceJournal` writer
persists them, but RK names no *builder* between raw evidence and the eight
traces. The Round 3 audit
(`../review/round3-consolidated.md`, F16) identifies this missing projector as the
common root cause of a cluster of producer gaps:

- **F11 — `action_trace` has no producer.** It is *not* a synonym for `tool_trace`
  (p.9 lists distinct ops: `read_file`, `edit_file`, `run_tool`, `write_report`,
  `update_task_state`, `inspect_diff`, `declare_complete`, tied to overall episode
  coherence). `ActionEvent` is referenced in the schema but never defined, so the
  H3 completeness gate ships empty or a wrong copy of `tool_trace`.
- **F12 — `context_trace` has no producer.** `recall()` returns a `String`, not
  `Vec<ContextEntry>`, and `influenced_decision` is unknowable at orient time. The
  H2 cross-session-recall gate is *defined on* `influenced_decision`, so it is
  unmeasurable and H2 is uncertifiable.
- **F13 — `tool_trace` / `recovered` and a `ToolStatus` contradiction.**
  `ToolEvent` lacks a producer for `recovered` (tool-recovery-rate is dead), and
  the `ToolStatus` enum is 3 variants in data-model §7 (the SSOT: `ok`/`error`/
  `blocked`) but 5 in PRD 03 (`ok`/`error`/`blocked`/`timeout`/`truncated`).
- **F14 — `verification_trace` has no `CheckResult→VerifyEntry` producer.**
  `method`/`covers`/`interpretation` are unmapped for non-deterministic checks, so
  the report degrades from requirement-linked evidence toward assertion.
- **F18 — per-turn intervention-log filter unspecified.** Nothing says which
  intervention records (by `source_message_id` / time-window) belong to *this*
  turn, so the embedded `intervention_log` over- or under-reports.
- **F19 — `episode_id` / `task_id` stability is implicit.** `episode_id =
  "ep_<task_id>"` is the lossless regrouping key (ADR-0018), but the mechanism
  holding `task_id` constant across a task's turns is unstated.

F15 belongs to the same surface: MCP's `McpToolFn::call` returns `String`, not
`ToolOutcome`, re-introducing the prefix-sniffing misclassification ADR-0022
abolished — for the whole MCP surface.

## Decision

Introduce a **`compose`-time episode-package assembly projector** that builds the
eight typed traces from raw evidence. It is the single named place where raw
events become typed traces, resolving the cluster above:

- **Define `ActionEvent`** and project `action_trace` from the agent's externally
  meaningful operations (distinct from `tool_trace`); project the `recovered`
  field and the `VerifyEntry` records at compose-time from the raw evidence.
- **`recall()` emits `Vec<ContextEntry>`** (not `String`), and the projector sets
  a **v1 `influenced_decision` heuristic** so the H2 recall gate is measurable.
- **Pin the per-turn intervention-log filter** — which records belong to *this*
  turn — so the package neither over- nor under-reports (F18), and pin the
  `task_id` stability mechanism that holds `episode_id` constant across a task's
  turns (F19).
- **Reconcile `ToolStatus`** to one set — {`ok`, `error`, `blocked`, `timeout`,
  `truncated`} — across data-model §7 and PRD 03, ending the 3-vs-5 contradiction.
- **Fix `McpToolFn::call` → `ToolOutcome`** (not `String`), restoring the
  structural status contract of ADR-0022 across the MCP surface.

Detail: `docs/prd/05-compose.md`, `docs/prd/04-observe.md`, `docs/prd/03-feed.md`,
`docs/prd/07-mcp.md`, `docs/architecture/data-model.md`.

## Consequences

- Every one of the eight `EpisodePackage` traces has a named producer, so the H3
  completeness gate ships real `action_trace` data and the H2 recall gate is
  measurable instead of structurally dead.
- `action_trace` stays distinct from `tool_trace`, preserving the episode-coherence
  diagnostic the paper ties to the action trace (p.9, Table 4).
- One `ToolStatus` set is authoritative across data-model and PRD 03;
  `McpToolFn::call` carries `ToolOutcome` structurally, so the prefix-sniffing
  misclassification ADR-0022 abolished does not re-enter through MCP, and MCP
  timeouts have a place to record (the F13 surface).
- `influenced_decision` is a documented v1 heuristic, not a silent guess, so the
  H2 gate's basis is explicit and can be refined without schema churn.
- The projector is the assembly half of the evidence pipeline; it is distinct from
  the eval-substrate ablation workstream (ADR-0035), which they share only at the
  `context_trace` producer.