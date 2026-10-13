---
category: Added
changelog: **BBP concurrency metrics.** `rusty_bbp::metrics` folds a task's log into revocations, discarded work, budget refusals, stale rejections, stale verdicts and diff overlap; `bbp metrics` prints them and `bbp mod` reports wall clock.
---
## 2026-10-13 - BBP concurrency metrics

- **Added:** `rusty_bbp::metrics::{of, overlap}`: pure folds over the log (no new event, no rule changed). `bbp metrics --dir D --task T` prints them as JSON; `Outcome::elapsed` and `bbp mod`'s `elapsed_ms` give the wall clock (the log has no timestamps). Definitions are those of the 2b gate criteria in `docs/bbp-concurrency-design.md`.
- **Test:** six scenario tests with known values (losing builder's work counted as discarded, three overlap levels, budget refusals vs stale writes, a stale verdict), and a host e2e where two scripted builders edit the same lines at `max_live_turns = 2` and the report shows one revocation and hunk overlap.
- **Known limitations:** model tokens are not visible to the core, so discarded work is in messages, reads and bytes; wall clock on scripted agents measures host overhead only. The scripted run checks the measurement, not the gate criteria; no evidence run has been made.
