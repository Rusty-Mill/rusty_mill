---
category: Added
changelog: The terminal core now keeps Kitty image placements on a headless grid and carries them in checkpoints, the first step of Kitty graphics through the mux.
---
## 2026-11-17 - C53 S1 headless image placements

- **Added:** `rusty_term_core` records a `Placement` (stored image id, cell footprint, scroll anchor) for each Kitty image placed with an id, bounded at 64, with `Grid::placements()`, `Grid::image_stamp()` and `Grid::placement_top_row()`. Placements scroll with their text and drop when the image is deleted or evicted, their rows leave scrollback, or the screen is cleared, resized or reset. RTCP carries them in an optional trailing section written only when there are placements, so checkpoints without images are byte-identical to before and v6 data still decodes. Sixel, iTerm2 and id-less Kitty images keep only their half-block cells.
- **Known limitations:** nothing sends placements to a client yet (S2, bit 52); a split checkpoint drops placements anchored in history it does not carry.
