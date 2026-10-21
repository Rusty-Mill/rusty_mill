---
category: Changed
changelog: CI tests each tree once per OS: PRs run Linux, the merge queue adds Windows and macOS and reuses a green PR's Linux result, and main has a daily sweep instead of a post-merge run.
---
## 2026-10-21 - CI runner budget

- **Changed:** `ci.yml` no longer runs on `push` to `main`; the merge queue
  already tested the tree that lands. The full sweep runs daily instead of
  weekly, which also keeps `main`'s caches fresh for PR runs.
- **Changed:** a `pull_request` run is Linux-only. Windows and macOS jobs run
  in the merge queue (and in manual and scheduled sweeps).
- **Changed:** a queue entry skips its Linux jobs when a green PR run already
  tested the identical tree on the same first parent, read from that run's
  `tested-tree` artifact. Entries queued behind others, and any failed lookup,
  run Linux as before.
- **Known limitations:** a Windows-only failure now shows up as a queue
  ejection, not a red PR. Run the workflow manually on the branch first when a
  change is Windows-sensitive. The `push`-to-`main` concurrency keys in the
  job definitions are now unused and left for a follow-up.
