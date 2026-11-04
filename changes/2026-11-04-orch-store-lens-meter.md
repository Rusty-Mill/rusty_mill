---
category: Added
changelog: "`orch-store` saves and loads a lens experiment's meter snapshot (`Store::save_meter`/`load_meter`) in a new `meters.mmap`."
---
## 2026-11-04 - orch-store: the lens meter is stored

- **Added:** `Store::save_meter(experiment, &Snapshot)` and `Store::load_meter(experiment)` (Let Em Loose #594). Each lens experiment keeps one meter record in a new `meters.mmap` in the state directory, under the same directory lock as goal snapshots. Restore the meter with `Meter::restore`, which charges every reservation still held.
- The record is keyed by a hash of the experiment name and also stores the name. A key that holds another experiment's meter, or a meter that lists a task twice, is refused as corrupt.
- **Format:** positional bincode tagged `rusty_orch::MeterRecord::v1`. Any layout change needs a new tag or a migrating reader. Don't roll rusty_orch back past this once a meter is saved.
- `orch-store` now depends on `orch-lens`, an in-workspace crate in the same family, for the `Snapshot` type. No new third-party dependency.
- **Unchanged:** existing goal and assignment records. An existing state directory gains an empty `meters.mmap` the next time it is opened.
