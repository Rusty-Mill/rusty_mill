---
category: Changed
changelog: Mill Board `card_add` and `card_take` no longer read the whole board; on the 1,425-card profile they read 2 KB and 45 KB instead of 3.7 MB each.
---
## 2026-11-10 - Mill Board writes without whole-board reads

- **Changed:** `card_add` takes its number from the store's reservation, above the last number this board reserved, and reads no other card: 3 requests and 2 KB instead of 11 and 3.7 MB. Numbers stay unique across boards and restarts and are never reused.
- **Changed:** cards made directly in rusty_tick without a number are numbered by the startup job (`number_cards`), by the first new card if that job failed, or on demand by the new admin tool `board_number_cards`; no longer on every add.
- **Changed:** `card_take` reads the work-queue columns by tag, then only the candidates it tries and their dependencies' tasks: 45 KB instead of 3.7 MB. Order, dependency waits, superseded skips and claims are unchanged.
- **Added:** `Store::in_columns`, `Store::open_among` and `Store::highest_reserved`, with full-scan defaults; `TickStore` overrides them.
- **Changed:** a `board_list` page stops at its limit before measuring the next card (review of #1612).
