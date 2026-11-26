---
category: Added
changelog: Mill Board cards gain a one-line `state` field, and writes are now bounded (title 200 characters, body and prompt 8 KiB, comment 2 KiB); existing larger cards are never rewritten.
---
## 2026-11-26 - Lean cards: state line and content budgets (Mill Board #1435)

- **Added:** an optional `state` field on a card (one line, at most 200 bytes), settable with an optional `state` argument on `card_update`, `card_comment` and `card_move`; a write that leaves it out keeps the old value. The `board_list` summary view carries it when set.
- **Changed:** title is at most 200 characters, body 8 KiB, prompt 8 KiB (was 32 KiB) and one comment 2 KiB. A write over a limit is refused with the field, the limit, the size sent and a hint to link the detail. A limit applies only to a write that changes that field, so a card already over a limit can still be edited and is never truncated or rewritten.
- **Known limitations:** `card_move` sets `state` in a second write after the move, so one call makes two events; a bad `state` is refused before the move. Comment kinds (#1436) and the agent view profile (#1437) are separate changes.
