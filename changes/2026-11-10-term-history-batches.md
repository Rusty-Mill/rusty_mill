---
category: Added
changelog: rusty_term_core can split a checkpoint from its history and add history batches newest first.
---
## 2026-11-10 - rusty_term_core: history batches for newest-first attach (C03 part 1)

- **Added:** `checkpoint::encode_split` returns a checkpoint without the older history plus that history as RTHB batches, newest first, each ending on a logical line. `Grid::prepend_history` adds a batch above the history a grid holds, rewrapped to its width, within the scrollback cap, without moving the view. Prompt marks travel with their rows.
- **Changed:** the Grid field audit test reads the encoder from `Grid::write_from`, which `Wire::write` now calls with no split.
- No wire, server or client change yet; the mux side follows in part 2 (card #1420).
