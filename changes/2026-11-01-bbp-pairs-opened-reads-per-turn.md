---
category: Fixed
changelog: bbp pairs reads each run's reads_per_turn from the opened task and refuses a run whose limit was extended.
---
## 2026-11-01 - bbp pairs: reads_per_turn from the opened task

- **Fixed:** `bbp pairs` took a run's `reads_per_turn` from the final card, after any human `extend`. A run opened at 30 and extended to 60 matched a manifest of 60, so a pair whose arms started under different limits could pass as comparable. The value now comes from the run's `TaskOpened` event, and a run with any `reads_per_turn` extension, even one back to the opened value, refuses (#1327).
- **Known limitations:** extensions of other budget fields are not refused. They do not enter the settings comparison, and the `overshoot` metric still compares spending with the extended `messages` and `bytes` limits, so an extension can hide an earlier overshoot.
