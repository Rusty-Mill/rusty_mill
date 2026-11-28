---
category: Added
changelog: The Let Em Loose tab's per-claim spend meter has a view model (spent of budget as a word label, a capped bar fill, an over-budget state). Not drawn yet.
---
## 2026-11-28 - LEL spend meter view model (slice 5)

- **Added:** `gui/lel_spend.rs`, a pure parser for the Mill Board `lel_spend` reply (`[{id, spent, budget}]`) and a `Meter` with a state word (within, over budget, no budget), a bar fill capped at 100 and a text label. A bad entry refuses the whole read so an overspend is never half-shown. Card #1416, slice 5 of `1416-lel-tab-slice-plan.md`.
- **Known limitations:** no UI and no live read yet; the lens list and spend share still wait on LEL Q42.
