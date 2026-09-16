# ADR-0020: Entropy categories — RK's 6 reconciled to the paper's 7

- Status: Accepted
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
- All seven paper categories (code, documentation, dependency, test, file
  residue, architecture, workflow) and the 0-3 severity scale are confirmed
  verbatim against the clean extraction (p.10); the PDF verification caveat is
  therefore lifted (Round 3 D1).
- The 6→7 map is lossless for entropy-delta: the paper's "code" and "file
  residue" fold into RK's `Residue` and "workflow" maps to `TaskContradiction`,
  with the remaining four 1:1. Because entropy-delta is the category-agnostic
  `−Σ severity`, the fold loses labeling granularity but no comparison data.
- RK keeps its six-variant `EntropyCategory` enum plus the map rather than
  splitting `Residue` back into code/file-residue (Round 3 D8); the split would
  add labeling granularity with no metric benefit.
