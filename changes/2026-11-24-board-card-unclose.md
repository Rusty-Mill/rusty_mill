---
category: Added
changelog: Mill Board `card_unclose` undoes a close by the merge watch; card #1620, part 3 of 3.
---
## 2026-11-24 - Mill Board `card_unclose`

- **Added:** `card_unclose` (needs `triage`) returns a card the merge watch closed to the column it was in, clears its ship time and keeps its evidence; a comment records who undid it. A card closed any other way, or not Done, is refused, and so is one an open card depends on that has started (claimed or in a work column); a dependent still waiting waits again. The check shares the gate every dependency-checked take holds.
- **Changed:** a merged pull request the card already lists as evidence is no longer a marker, so an unclosed card is not closed again by the same PR.
- **Changed:** a card closed by the merge watch carries a typed `merged` record (the column it was in, and when): an optional field on the card, a reserved `merged:<ms>:<column>` tag in rusty_tick. No migration: older closes simply have none and cannot be unclosed.
- **Known limitations:** `auto_close` stays off by default; turning it on is the owner's separate decision (ADR-0008).
