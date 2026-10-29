---
category: Changed
changelog: Mill Board's card_assign re-files Done cards into another project; they stay Done (needs triage or close).
---
## 2026-10-29 - Mill Board: re-file Done cards into another project

- **Changed:** `card_assign` no longer refuses Done cards. It changes only the project; the
  card stays Done and its rusty_tick task stays closed (it moves to the new project's list).
  A Done card needs `triage` or `close`; other cards still need `triage` or `project`. The
  reassignment is recorded as an `Assigned` event.
- **Known limitations:** none known; moving a card out of Done is still refused.
