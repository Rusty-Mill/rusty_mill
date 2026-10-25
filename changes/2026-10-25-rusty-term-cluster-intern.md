---
category: Changed
changelog: rusty_term attaches combining marks with one hash lookup and no allocation once a continuation is known, and builds the grapheme table for a frame only when the table has grown.
---
## 2026-10-25 - rusty_term: combining-mark interning

- **Changed:** `Grid` keeps a std `HashMap` from each interned grapheme continuation to its id. A combining mark is looked up in it instead of scanned for in the table. The grown continuation is built in a reused buffer, so a mark whose continuation is already interned allocates nothing. Previously each mark cloned a `String` and searched the table linearly.
- **Changed:** the `Arc` copy of the grapheme table that `snapshot_dirty` and `snapshot_viewport` hand out is built on the first frame after the table grows, not on every insert. N new continuations between two frames now cost one table copy, not N. Frames still share one `Arc` while the table is unchanged.
- **Changed:** both restore paths (checkpoint and RTSN snapshot) rebuild the index, and checkpoint decoding charges its memory to the allocation budget.
- **Changed:** instructions per byte: unicode_heavy 115.5 to 102.2; the other workloads do not move. The unicode_heavy ceiling in `bench/ir_ceilings.txt` drops from 128 to 112.
- **Tested:** equal continuations share an id, including stacked marks. Frames share the table until it grows, and a frame already handed out keeps its table. After a checkpoint or snapshot restore, a known continuation reuses its id instead of being appended again.
- **Known limitations:** on unicode_heavy, where about 9% of code points are marks, the wall-clock change is within this VM's noise (median 34.7 vs 34.5 MB/s, best 39.6 vs 42.6, 30 paired runs). On a 4 MB stream of accented vowels it is clear: median 16.8 to 22.8 MB/s (+36%, 12 paired runs). The index holds a second copy of each continuation, bounded by the 8192-entry table.
