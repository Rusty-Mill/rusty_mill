---
category: Added
changelog: Mill Board gains an optional Parked column kind for cards set aside outside active work.
---
## 2026-11-16 - Mill Board: Parked column

- **Added:** column kind `parked` (fixed id `parked`, at most one, optional). Cards in it are never handed out by `card_take`, sit in the roadmap's Later lane, and are outside the gate order (leaving it counts as starting from the beginning). The default workflow adds a Parked column with Backlog/Ready -> Parked and Parked -> Backlog/Ready moves (Triage). An existing `workflow.json` is unchanged: add the column in Settings > Columns (kind Parked).
- **Known limitations:** an older binary rejects the `parked` kind once an admin adds the column; remove it before rolling back. Not the same as the `deferred` project status.
