---
category: Added
changelog: **BBP per-state wall timing.** `bbp mod` prints `state_ms`, the ordered wall-clock time of each state visit, so the spec-authoring wall time (the first Planning visit) can be compared across `--live-turns` settings; this backs 2b gate criterion 3.
---
## 2026-10-14 - BBP per-state wall timing

- **Added:** `moderator::Outcome.visits` (ordered `(State, Duration)`, a revisit is its own entry) and `state_ms` in the `bbp mod` JSON, e.g. `[["Planning",41200],["PlanGate",300],...]`. The spec-authoring wall time of a run is the first `Planning` entry.
- **Backs:** 2b gate criterion 3 (wall-clock benefit): spec authoring wall time at N=2 against N=1.
- **Known limitations:** the log carries no timestamps, so the time is the moderator's clock and its resolution is one poll (`--poll-ms`, 500 by default); a state shorter than a poll may not appear. A gate's time is the human's. Not an evidence run.
