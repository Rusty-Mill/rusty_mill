---
category: Added
changelog: **BBP spec_conflicts metric.** `rusty_bbp::spec_metrics::spec_conflicts` compares each non-approved spec version with the approved one on a common base by changed lines (a first-party Myers diff), and `bbp metrics` reports it; this backs 2b gate criterion 4.
---
## 2026-10-14 - BBP spec_conflicts metric

- **Added:** `spec_metrics::spec_conflicts(events, blob_reader)`: for every spec version other than the one the human approved at the plan gate, the common base (the latest spec stored before either, so both authors could have read it) and an outcome: `Conflict` (both edited a common base line, or inserted at one point, differently), `Clean` (different lines, touching lines, or identical edits), `ParallelRewrite` (no earlier version, so no base; never a conflict), or `Unmeasured` (unreadable spec or beyond the exact diff limit). `bbp metrics` prints it as `spec_conflicts`. New first-party dependency `rusty_diff` (foundation, no dependencies); `codec::spec_markdown` reads a stored spec.
- **Backs:** 2b gate criterion 4 (contention is real). The gate counts versions that are both `Conflict` and *unused*; the unused flag comes from the retained-versus-unused attribution, a separate change.
- **Known limitations:** touching but non-overlapping edits are treated as different lines, as is a pair that did not both read the same base (the latest earlier version is assumed). A spec edited by building on the other version shows as a conflict until the attribution marks it retained. Not an evidence run.
