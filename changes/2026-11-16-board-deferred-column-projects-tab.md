---
category: Added
changelog: Mill Board gains an optional Deferred column for parked cards, and Settings splits Projects from Repositories with a Link repository action.
---
## 2026-11-16 - Mill Board: Deferred column and a Projects settings tab

- **Added:** column kind `deferred` (fixed id `deferred`, at most one, optional). Cards parked there are never handed out by `card_take`, sit in the roadmap's Later lane, and are outside the gate order (leaving it counts as starting from the beginning). The default workflow adds a Deferred column with Backlog/Ready -> Deferred and Deferred -> Backlog/Ready moves (Triage). An existing `workflow.json` is unchanged: add the column in Settings > Columns (kind Deferred).
- **Changed:** Settings > Repositories now holds repositories and their GitHub slugs; the new Settings > Projects tab holds projects, their status and their repository links (Link repository, Unlink). Both edit one draft and save the same registry; no server or file format change.
