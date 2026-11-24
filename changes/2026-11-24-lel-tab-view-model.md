---
category: Added
changelog: LEL tab view model in rusty_term parses the Mill Board decision inbox (no UI yet).
---
## 2026-11-24 - LEL tab view model

- **Added:** `rusty_term` gets `gui/lel.rs` (#1416, slice 1), a pure parser from the Mill Board `inbox_list` reply to display rows, with a golden fixture and a drift test in `rusty_mill_board`. `rusty_json` becomes an optional `gui` dependency (approved by Nano, 2026-11-24). No UI and no wire change yet.
