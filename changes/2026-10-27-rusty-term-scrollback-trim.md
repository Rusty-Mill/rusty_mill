---
category: Changed
changelog: rusty_term frees the trailing blanks of scrollback lines while the terminal is idle, so a full history of short lines takes about a tenth of the memory it did.
---
## 2026-10-27 - Idle scrollback blank trimming

Each history line stored every cell of its width, so 10,000 lines at 200 columns held 15.9 MB however short the text was. `Grid::compact_history(budget)` (new public API, from the approved design for backlog row 17) now stores a hard-broken line's trailing run of one padding cell as that cell plus a repeat count. `Line` stays 32 bytes. Readers, reflow, snapshots and checkpoints see exactly the same cells. Soft-wrapped lines are left alone.

Compaction runs off the parse path. The TUI reactor and the GUI window each keep a small `Compactor` per pane: 100 ms after the last output or resize it trims up to 1,000 lines per step, and any write to a line stores its run again first. A resize reflows history at full width, so it schedules a pass even when no output follows. Measured with the ignored `measure_history_memory` test: short lines went from 15.9 MB to 1.7 MB, and full-width text stays at 15.9 MB. The IR gate passes. `alt_screen_flicker` goes from 49.7 to 51.1 instructions per byte (ceiling 55), and every other workload moves under 1%.
