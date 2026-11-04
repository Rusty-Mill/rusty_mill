---
category: Added
changelog: "`orch_lens` meter snapshots and crash-safe restore; `call` saves the meter after each reservation, before any provider is asked."
---
## 2026-11-03 - orch-lens: the meter survives restarts

- **Added:** `Meter::snapshot` and `Meter::restore` (Let Em Loose #594, slice-4 review: one experiment meter and its reservations across restarts). A snapshot holds the settled spend, each task's spend and calls, and each reservation held. Restore settles every held reservation at its amount and counts it as a call. A call in flight at a crash may have reached its provider, so a restart errs toward stopping. The ceilings come from config and are not stored.
- **Added:** `Meter::release`, which drops a reservation for a call that never started.
- **Changed:** `call::Ctx` gains `save`, which is run after each reservation and each settlement. A reservation that cannot be saved is released and ends the job as `End::Unsaved`, with no provider asked. A settlement that cannot be saved keeps an answer and reports it in `Called::unsaved`, so the shell can stop. After a passed-over provider it ends the fallback as `End::Unsaved`, so no further provider is asked. This change is in-family and has no caller outside `orch-lens` yet.
- **Unchanged:** where the snapshot is stored. The `orch-store` record comes with the `lens run` shell.
