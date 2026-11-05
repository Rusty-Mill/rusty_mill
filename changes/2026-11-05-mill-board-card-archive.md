---
category: Added
changelog: Mill Board archives Done cards: card_archive, card_restore, card_archive_done, board_list `archived`/`done_days`, and an Archive view.
---
## 2026-11-05 - Mill Board card archive

- **Added:** Done cards can be archived. `card_archive` and `card_restore` (need `close`) and the bulk
  `card_archive_done` (needs `triage`) keep an `archived:<ms>` or `restored:<ms>` tag on the card's rusty_tick
  task, beside `number:<n>`, so there is no migration. Restoring starts a card's Done window again without
  changing its ship time; the bulk archive checks each card again as it commits. An archived card is still Done: `card_get`, the roadmap,
  metrics and dependencies see it as before.
- **Changed:** `board_list` leaves out Done cards that are archived or were done more than 14 days ago.
  `archived: "include" | "only"` brings them back or lists only them, and `done_days` changes the window
  (`0`: only archived cards are left out). The web UI's Done column does the same and links to a new
  Archive view (search by number, title or tag; Restore), with "Archive all" and a dialog Archive/Restore button.
- **Known limitations:** the 14-day window is a `board_list` argument, not a board setting; the UI uses the
  default. Archiving open cards (won't-do) is out of scope.
