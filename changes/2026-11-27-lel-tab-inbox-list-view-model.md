---
category: Added
changelog: Let Em Loose tab inbox list and top-strip chip view model in rusty_term (#1416 slice 3, not yet drawn).
---
## 2026-11-27 - LEL tab inbox list and chip view model

- **Added:** `gui/lel_inbox.rs` turns the slice 1 inbox rows into list items (title, candidate count or task-level stop, host reason, irreversible mark), the chip text of D53/L3c ("2 decisions waiting", "No decisions waiting"), and a wrapping keyboard selection that follows the escalation by id across a refresh (D60). No UI, no I/O, no board change.
- **Known limitations:** Nothing draws the chip or list yet; the live feed waits for the Mill Board read endpoints (D173, gated). The "Dry run attached" line of L2a needs slice 4's action data.
