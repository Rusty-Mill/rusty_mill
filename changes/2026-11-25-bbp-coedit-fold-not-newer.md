---
category: Changed
changelog: rusty_bbp refuses a co-edited version whose id is not newer than the spec's newest, and names why a logged version did not apply.
---
## 2026-11-25 - rusty_bbp co-edit fold refuses an out-of-order version

- **Changed:** `Doc::apply` refuses a version id that is not newer than the newest version that inserted a line (`Unapplied::NotNewer`). Before, an older id was accepted and its lines landed elsewhere than the edit meant, and a reused one was reported as a missing line (#1861 review follow-ups, #430).
- **Changed:** `Missing` is renamed `Unapplied`, and `Doc::apply` returns it. `TaskState::unapplied` records `NotNewer` as well as a missing line or base, and the engine's `ref_unresolved` message says which.
- **Known limitations:** only a hand-made or out-of-order log can hit this; the engine writes ids in log order. A version that only deletes does not raise the floor.
