---
category: Changed
changelog: Mill Board keeps board-document fields it does not know when it rewrites a card, so rolling back to it after a newer release loses nothing.
---
## 2026-11-12 - Mill Board keeps unknown board-document fields

- **Changed:** when the board rewrites a card's board document in rusty_tick, it writes back unchanged any field it does not know. A newer release's fields, such as the planned `externalRefs` (ADR-0006), then survive a rollback to this release. Documents without such fields are written exactly as before, and the 64 KiB size check counts the kept fields.
