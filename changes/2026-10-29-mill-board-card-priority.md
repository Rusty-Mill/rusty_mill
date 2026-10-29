---
category: Added
changelog: Mill Board's web UI shows each card's priority, sorts by it, and sets it from the card dialog.
---
## 2026-10-29 - Mill Board card priority in the web UI

- **Added:** cards on the board and roadmap show a High, Medium or Low badge
  (rusty_tick's priority 5, 3 or 1; no priority shows nothing). Each column
  lists the highest priority first, the order `card_take` already uses.
- **Added:** the card dialog has a priority picker that calls `card_plan`,
  so it needs Triage like the MCP tool. A change made elsewhere updates the
  open dialog. `tests/ui/priority.mjs` checks this in a browser.
- **Known limitations:** no rusty_tick or API change; priority was already
  stored as the tick task's own `priority` and settable over MCP with
  `card_plan`. `card_add` still creates cards with no priority.
