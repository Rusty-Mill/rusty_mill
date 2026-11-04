---
category: Added
changelog: "`rusty_orch lens run` saves each task's scheduler memory, so a restart does not re-ask passes already made."
---
## 2026-11-04 - rusty_orch: the lens runner's memory survives a restart

- **Added:** `orch_lens::saved`: `State::saved` turns the scheduler's memory into plain data and `State::restore` rebuilds it, re-checking every provider id and refusing a repeated entry (#594). Jobs in flight are not kept, since no job outlives its process.
- **Added:** `orch_store::Store::load_lens_state` and `save_lens_state`, one `LensStateRecord::v1` per BBP task in a new `lens-states.mmap` beside the meters. A new file, so nothing existing is migrated.
- **Changed:** `rusty_orch lens run` loads the task's memory at start and saves it after every lens's round; a failed save ends the run with an error. `passes` takes the `keep` callback. A crash mid-round asks that round's jobs again; the meter already charges any call it had reserved.
