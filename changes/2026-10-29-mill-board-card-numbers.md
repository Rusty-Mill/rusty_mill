---
category: Added
changelog: Mill Board cards get a short board-wide number (#42), shown on the card and accepted by every MCP tool that takes a card id.
---
## 2026-10-29 - Mill Board card numbers

- **Added:** every card has a board-wide number, handed out in creation order across all
  projects and kept when the card moves to another project. It is stored on the rusty_tick task
  as the reserved tag `number:<n>`, so it lands in the same request that creates the task and
  needs no rusty_tick change. The web UI shows `#n` in the card's corner and in its dialog (the
  linked GitHub issue now reads `issue #n`). Every MCP tool that takes a card id also takes
  `#42` or `42`, including `card_take` and `card_plan` dependencies. Cards without a number
  (older cards, or tasks tagged `agent` directly in rusty_tick) are numbered oldest first when
  the board starts and before the next new card.
- **Known limitations:** if the highest-numbered card is deleted in rusty_tick, its number can
  be handed out again. `number:` is now a reserved tag that `card_update` refuses.
