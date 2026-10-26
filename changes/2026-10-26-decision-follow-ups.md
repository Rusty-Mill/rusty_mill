---
category: Added
changelog: remind-me schedules a follow-up review for each new decision and records whether it held up (held, revised, reverted).
---
## 2026-10-26 - Decision follow-ups: did it hold up?

- **Added:** a new decision gets a review due 21 days later (`REMIND_ME_DECISION_REVIEW_DAYS`, `0` to turn off; `metadata.stakes: "low"` skips it). Due reviews appear in `remind_me_digest` and the session brief. `remind_me_resolve` takes `review: held | revised | reverted` and a note, kept apart from `outcome`, so "carried out" and "held up" stay two things. `remind_me_stats` reports decisions by review state, per project and month. ADR-0030.
- **Known limitations:** decisions written before this change are not backfilled and report as `unscheduled`. The review is stored in `metadata`, so an update that replaces the whole metadata drops it.
