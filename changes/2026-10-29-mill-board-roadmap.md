---
category: Added
changelog: Mill Board shows a per-project roadmap (now, next, later, blocked, shipped) derived from its cards, in the web UI and as the board_roadmap tool.
---
## 2026-10-29 - Mill Board per-project roadmaps

- **Added:** a Roadmap toggle in the Mill Board web UI shows the chosen project's cards in five lanes: now
  (building, qa, review), next (ready), later (backlog), blocked, and shipped (done in the last 30 days, newest
  first). Lanes are ordered as `card_take` orders work, and a card waiting on unfinished dependencies says so.
  It follows board changes live and is remembered in the browser. The same roadmap is the `board_roadmap`
  tool (MCP and `POST /api/board_roadmap`), with trimmed cards, scoped like `board_list`.
- **Added:** cards carry a read-only `done_ms`, the time they shipped: rusty_tick's `completedMs` for tasks
  closed as done, and the move into Done otherwise. Tasks closed as won't-do have none and are not shown as
  shipped. No rusty_tick change.
- **Known limitations:** one project at a time; no milestones or target dates.
