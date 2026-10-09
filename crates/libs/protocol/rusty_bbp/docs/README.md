# rusty_bbp design record

The Blackboard Protocol's specification and the review rounds that shaped it, moved here from `baileyrd/rusty_bbp` when that repository was archived. The code that implements the spec is the crate one level up; the trace catalog its tests reproduce is [`../TRACES.md`](../TRACES.md).

## Specification

| File | What |
| --- | --- |
| [`spec/bbp-v0.4-implementation-spec.md`](spec/bbp-v0.4-implementation-spec.md) | The spec the crate implements (v0.4.1 errata applied). |
| `spec/bbp-v0.3-draft-design.md`, `v0.2`, `v0.1`, `v0` | Earlier drafts, kept for the review history. |

## Review

| File | What |
| --- | --- |
| `review/v0-review.md`, `review/v0-merged.md` | Round one: the single integrated review and the merged findings of the three model reviews. |
| `review/v0.N-review-*.md`, `review/v0.N-merged.md`, `review/v0.N-review-prompt.txt` | Rounds two to four: each model's review verbatim, the merged findings, and the prompt they answered. |
| [`review/v0.4-implementation-plan-review.md`](review/v0.4-implementation-plan-review.md) | Review of the staged implementation plan; its amendments (A1, A3, A4) are cited in the code. |
| [`review/sovereignty.md`](review/sovereignty.md) | The dependency audit that made the crate Tier S. |
| [`review/orch-core-relationship.md`](review/orch-core-relationship.md) | How this protocol relates to `rusty_orch`: compose above, retire the orch Board at BBP stage 3. |
