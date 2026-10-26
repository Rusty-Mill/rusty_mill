---
category: Changed
changelog: rusty_term stores each cell in 8 bytes over a per-grid style table, cutting grid memory about 3x (200x50 with 10k lines of history goes from 47.7 MB to 15.9 MB) with no change to checkpoint or snapshot bytes.
---
## 2026-10-25 - rusty_term: 8-byte cells over a style table

- **Changed:** a stored cell is a `PackedCell`: the character, the wide-trailer and DECSCA bits, and a `StyleId` into the grid's `StyleTable` (colours, attributes, link, cluster). Equal styles share one id. Readers still get `Cell` by value through `Grid::row`, `row_cells` and the frame and view paths. This is PR 2 of the compact cell plan (backlog row 5).
- **Changed:** styles are reclaimed by a sweep that marks every retained cell. It runs between parser batches when the table has doubled since the last sweep or history has turned over, and after a clear of history, a full reset or a resize. Compaction renumbers the table once at most 65,536 styles are live. A small cache in front of the table's index keeps the per-write cost flat.
- **Changed:** RTCP and RTSN bytes are unchanged; the hashes pinned by #1079 still match. RTCP encode at 200x50 + 10k drops from 19.7 to 7.9 ms. The RTCP decoder charges each reserved style for both its wire table and the grid's (`size_of::<Cell>() + 74` bytes).
- **Changed:** RTSN decode rejects a snapshot with more than 2^22 distinct grid styles.
- **Added:** `Grid::style_stats` (hidden) for memory tests; `bench/gen_workloads.py` gains `sgr_alternate` (three pens alternating within each row), generated last so the other workloads' bytes are unchanged.
- **Tested:** style table unit tests; sweep and compaction keeping every colour; budget charges equal for writer and reader on random grids; a decoder cap test; `tests/memory.rs` (8 bytes a cell, under 20 MB at 200x50 + 10k, a style per cell bounded and released). Ignored measurements report RSS, pauses and encode time.
- **Known limitations:** output with a distinct truecolour style in every cell costs up to about 170 B/cell and can stall the grid for 120 ms (200x50 + 10k) to 490 ms (4.19M cells) while the style index grows. The owner accepted both on 2026-10-25.
