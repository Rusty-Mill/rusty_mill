---
category: Added
changelog: **BBP 2b run inputs frozen.** Ten paired tasks (five separable, three of them null pairs; five shared-section), a pilot task, rubrics, pass marks, rosters and model-pinned agents files, approved by the owner.
---
## 2026-10-21 - BBP 2b evidence run inputs

- **Added:** `docs/bbp/2b-run/`: briefs and failing tests for p01 to p10 and an uncounted pilot, `tasks.json` (classification, concerns, section ownership, the concern each rubric item needs), one rubric and pass mark per pair in `rubrics/`, the control roster and a treatment roster with `planner2`, and agents files that pin every role to `claude-sonnet-5-5`, load no settings source, disallow `Agent` and `Task`, and stop on `stale_turn`. `bbp agents-check` accepts the pair.
- Every task's tests pass against a reference implementation written for the check (not committed), so each task is passable as briefed.
- Approved by the owner on 2026-10-21. Frozen: a run whose set differs is void. No run is queued; the pilot and its spend need separate owner authorization.
