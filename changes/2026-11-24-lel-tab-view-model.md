---
category: Added
changelog: LEL tab view model in rusty_term parses the Mill Board decision inbox (no UI yet).
---
## 2026-11-24 - LEL tab view model

- **Added:** `rusty_term` gets `gui/lel.rs` (#1416, slice 1), a pure parser from the Mill Board `inbox_list` reply to display rows, with a golden fixture and a drift test in `rusty_mill_board`. `rusty_json` becomes an optional `gui` dependency (approved by Nano, 2026-11-24). No UI and no wire change yet.
- **Changed:** an entry whose `claim`, `candidates`, `reversible`, `stakes` or `card` is present but malformed refuses the read (null or absent is fine). `Row` leaves out `acted`, `totals`, `pause` and `evidence` for now; slices 3 and 4 add what the inbox and its actions need (`acted` matters before Reject or Choose). Names and text are raw: the renderer strips control and bidi characters. A change to the golden fixture now also selects `rusty_mill_board` in CI (`SHARED_FILES` in `affected_crates.py`).
