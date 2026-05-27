# ADR-0020: Entropy categories — RK's 6 reconciled to the paper's 7

- Status: Proposed
- Date: 2026-05-27
- Tags: faithfulness, entropy, observe

## Context

PRD 04 defines six `EntropyCategory` variants (`Residue`, `TestWeakening`,
`StaleDocs`, `DependencyChurn`, `BoundaryViolation`, `TaskContradiction`). The AI
Harness Engineering paper enumerates seven entropy categories. The two sets do
not line up one-to-one (consolidated plan §G), so entropy-delta comparisons
against the paper need an explicit reconciliation.

## Decision

Provide a paper-to-RK category map and document the reconciliation: the paper's
"code" category is merged into RK's `Residue`, and the paper's "workflow"
category is renamed to its RK equivalent. The authoritative category map and the
0-3 severity scale live in `docs/prd/04-observe.md`, with the faithfulness map in
`docs/ARCHITECTURE.md`.

## Consequences

- Entropy findings can be translated to the paper's seven categories for
  comparison without changing the RK enum.
- Status is Proposed pending human confirmation of the exact seven paper
  categories and the 0-3 severity thresholds against the rendered PDF (see the
  consolidated plan's PDF verification caveat).
