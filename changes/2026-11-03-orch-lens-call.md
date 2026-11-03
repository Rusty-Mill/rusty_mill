---
category: Added
changelog: "`orch_lens::call` runs one lens job over its providers in fallback order, fitting the context and reserving the worst case on the meter before each ask."
---
## 2026-11-03 - orch-lens: one job's call, with fallback and metering

- **Added:** `orch_lens::call::call` (Let Em Loose #594). It is pure over the `Model` port, through a new `Models` lookup trait, and the experiment's one `Meter`. For each provider in the job's order it fits the context to that provider's window and clearance, reserves `worst_case` and asks. It then settles what the reply cost.
- A provider that is not configured or not built, whose window the context cannot fit, or that answers `Unavailable` is passed over and the reason recorded. The experiment ceiling passes over paid providers only, so free ones go on. A task ceiling ends the job before any call. `Permanent` and `Transient` end the job on that provider and are not retried elsewhere.
- Output is capped at `MAX_OUTPUT_TOKENS` (4096) on every provider. A reply over the cap is refused as `Permanent`, and its cost is still settled. A failure with unknown usage settles the reserved worst case, so the meter errs toward stopping.
- **Unchanged:** nothing calls it yet. The `rusty_orch lens run` shell comes next.
