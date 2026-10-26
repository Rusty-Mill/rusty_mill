---
category: Added
changelog: Mill Board cards get a priority and dependencies; `card_take` takes higher priority first and skips waiting cards (ADR-0012, step 2).
---
## 2026-10-26 - Mill Board: priority, dependencies and reserved tags (ADR-0012 step 2)

- **Added:** cards carry a `priority` (0, 1, 3 or 5, rusty_tick's scale) and `deps` (cards that must be Done first), both set by the new `card_plan` tool, which needs Triage. `card_take` still looks nearest Done first; within a queue it now takes the highest priority, then the oldest, and skips a card whose dependencies are not Done. Naming such a card explicitly is refused with the reason.
- **Changed:** the tags `agent`, `board:…` and `claim:…` are reserved for the board's rusty_tick mapping. `card_add` and `card_update` refuse them, and the GitHub sync drops such labels when it imports an issue instead of carrying them over.
- **Known limitations:** the web UI does not show or edit priority and dependencies yet. Existing cards read as priority 0 with no dependencies.
