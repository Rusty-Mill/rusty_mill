---
category: Changed
changelog: The rusty_term interface font (Instrument Sans) now ships with the mux-client chrome (arc 2) instead of as a separate redesign slice.
---
## 2026-11-06 - rusty_term interface font moves to arc 2

- **Changed:** design pack D59 (owner decision, 2026-11-06). The GUI chrome is drawn on a character
  grid, so a proportional interface font needs real text layout. That layout is built once, for arc
  2's new top strip, block headers and badges. The current chrome redesign ends at slice 3, and the
  chrome keeps its monospace UI font until then. Docs only: STYLEGUIDE and gui-design DECISIONS.
