---
category: Added
changelog: Mill Board admins add, relabel, reorder and remove board columns and edit the moves between them (Settings > Columns, workflow.json, board_columns).
---
## 2026-11-01 - Mill Board: admin-managed columns

- **Added:** the board's columns and transition table are data an admin edits in Settings > Columns, kept in
  `workflow.json` beside the other settings. Each column has a kind (queue, work, or the fixed backlog, done and
  blocked) that decides how `card_take`, claims, the roadmap and metrics treat it. A move can be a gate that also
  needs `admin`, even with movement rules off. `board_columns` lists columns and moves for agents and the UI.
- **Changed:** rusty_tick's column history (`/api/v1/moves`) records any valid `board:<id>` column, not only
  the original seven, so time in an admin-added column is tracked. Deploy rusty_tick with the board.
- **Changed:** the default workflow is the previous seven columns and table, so nothing changes until an admin
  edits it. `board_list` and `card_move` take any column id the workflow has.
- **Known limitations:** removing a column that still holds cards is refused; move them first. A card whose
  column is missing from the workflow (edited by hand in rusty_tick) reads as backlog. Column ids are fixed once
  created; only labels change.
