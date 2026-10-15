---
category: Changed
changelog: **BBP 2b gate criteria, second revision (proposal).** The criteria now state that the run cannot exercise the stale-verdict gap, treat `unused_conflicts` as an upper bound, add variance, measurement-error and token-cost conditions, and propose a token cap rule and a log-revision-at-grant change (unbuilt).
---
## 2026-10-15 - BBP 2b criteria: second revision

- **Changed (docs):** `docs/bbp-concurrency-design.md`. Criterion 6 becomes a build precondition (the run has one tester, so the gap cannot show); criterion 3 gains null pairs and a poll-error bound; criterion 4 reads `unused_conflicts` as an upper bound confirmed by hand; new criterion 7 (token cost per outcome). Proposals, not built: a two-tester roster variant (recommended against), `at_rev` on `TurnGranted` with verdict binding, and a token-cap rule.
- **Second pass (2026-10-15):** defines separable and shared-section, puts the three null pairs first as a noise gate (stop and report outside 0.85 to 1.15), commits rubrics before the run with a mechanically blind grader, records who answered each gate (`self_approved` unless the declared owner), and orders the decision table (first match wins).
- **Changed (doc comments):** `unused_conflicts` and its report field say upper bound. No behaviour change. The criteria remain a proposal awaiting approval; no run is authorized.
