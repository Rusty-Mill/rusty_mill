# ADR-0019: Intervention model maps UI actions to avoidability / harness_gap / burden

- Status: Accepted
- Date: 2026-05-27
- Tags: faithfulness, mhir, observe

## Context

PRD 04 defines seven intervention "kinds" (`task_override`, `manual_reflect`,
`manual_groom`, `manual_verify`, `unverified_followup`, `tool_block`,
`direct_edit`). These kinds are a Rusty Keys invention; the paper instead
characterises an intervention by whether it was avoidable, whether it reflects a
harness gap, and the burden it imposed. As recorded today the log captures raw
HIR, not M-HIR, because it counts all interventions rather than the
missing-harness ones (consolidated plan §G). ADR-0014 defines M-HIR but does not
distinguish avoidable interventions.

## Decision

Add `avoidability`, `harness_gap`, and `burden` fields to the intervention record
and map each RK UI action onto them, so M-HIR counts missing-harness
interventions rather than every human action. The field definitions and the
RK-kind-to-paper-field mapping live in `docs/prd/04-observe.md`; the record
schema lives in `docs/architecture/data-model.md`.

## Consequences

- M-HIR's denominator/numerator semantics align with the paper's missing-harness
  intent rather than raw human-action counts.
- The seven RK kinds are retained as the UI-facing taxonomy but each carries the
  three paper-aligned attributes.
- The three field names (avoidability + burden level + harness gap) are confirmed
  verbatim against the clean extraction (p.10); the PDF verification caveat is
  therefore lifted (Round 3 D1).
- The M-HIR numerator is now **avoidable-only** (Round 3 D2): a correct
  `tool_block` is `unavoidable` — the policy working as intended is the harness
  *working*, not "support the human would otherwise have to provide" (p.4) — so it
  no longer counts toward M-HIR. This resolves the prior self-contradiction where
  an `unavoidable` block was both "not a missing-harness signal" and counted. The
  field definitions and the avoidable-only numerator live in
  `docs/prd/04-observe.md`; the record schema lives in
  `docs/architecture/data-model.md`.
