---
category: Fixed
changelog: **BBP moderator: a wait error while draining still kills every harness left.** Follow-up to the end-of-turn grace (#830).
---
## 2026-10-21 - BBP moderator: drain kills on a wait error

- **Fixed:** `drain_harnesses` (the end-of-run grace from #830) returned early on a `try_wait` error after moving harnesses out of its list, so those were dropped without being killed. They now stay in the list and are all killed before the error is returned, as the reap loop already did.
- No behavior change when waiting succeeds.
