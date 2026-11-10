---
category: Changed
changelog: Mill Board resolves a card number through its `number:<n>` tag in rusty_tick instead of reading the whole board.
---
## 2026-11-10 - Look up a card by number without reading the whole board

- **Changed:** `card_get #N` and every other `#N` reference on a rusty_tick-backed board read the `number:<n>` tag and that one card (about six small requests), not every task, board document and comment. The cost no longer grows with the board. `Store::numbered` scans `list` by default; `TickStore` overrides it. No rusty_tick change, so no deploy order.
- **Known limitations:** a number edited onto two cards by hand is still refused rather than guessed.
