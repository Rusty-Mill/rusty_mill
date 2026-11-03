---
category: Added
changelog: A merge-queue reaper workflow cancels CI runs for queue entries GitHub has already discarded.
---
## 2026-10-26 - Cancel CI runs for discarded merge-queue entries

- **Added:** `.github/workflows/merge-queue-reaper.yml`. When the merge queue rebuilds, GitHub deletes the discarded entries' `gh-readonly-queue/*` refs but leaves their CI runs going. From 2026-10-23 to 2026-10-26, 34 such runs used at least 1,450 runner-minutes after they could no longer merge, holding hosted runners that live entries were waiting for. On every merge-queue CI request, the reaper now cancels any in-flight merge_group CI run whose ref no longer exists. It uses the preinstalled `gh` CLI, only `actions: write`, no third-party actions, and runs main's copy via `workflow_run`. It is not a required check and never blocks or ejects an entry. If the queue state can't be read, it cancels nothing.
- **Changed:** `test_ci_workflow.py` now checks that no other workflow *triggers* on `merge_group`, rather than that none mentions it, so the reaper can filter on it.
- **Known limitations:** removing the last entry from the queue requests no new CI run, so that entry's run is reaped only at the next queue event. The reaper waits for a hosted runner like any other job.
