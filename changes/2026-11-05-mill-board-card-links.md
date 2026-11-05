---
category: Added
changelog: Mill Board cards can be marked superseded by newer cards or related to others; card_take never hands out a superseded card.
---
## 2026-11-05 - Mill Board: "superseded by" and "related" card links

- **Added:** `card_plan` sets `superseded_by` (newer cards that replace this one) and `related`, alongside `deps`. Each list replaces the current one and `[]` clears it. card_get and board_list show them.
- **Added:** `card_take` never hands out a card with a `superseded_by` link; named explicitly, it says to close the card or re-scope it and clear the link.
- **Changed:** `Board::plan` and `Card::plan` take a `Plan` struct instead of positional priority and deps.
- **Known limitations:** links live in the board's per-task document in rusty_tick, like `deps`, so no rusty_tick schema change or migration is needed, and rusty_tick's own UI does not show them. The reverse `supersedes` view, a UI display and the overlap helper are not built.
