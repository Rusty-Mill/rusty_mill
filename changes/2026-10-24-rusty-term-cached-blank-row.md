---
category: Changed
changelog: rusty_term blanks scrolled-in and erased rows by copying one cached blank row instead of a per-cell fill, about 30% faster on scroll-heavy output.
---
## 2026-10-24 - rusty_term cached blank row

- **Changed:** `Grid` keeps one prebuilt row of blank cells, and `Line::clear` copies it in a single `memcpy` instead of storing each cell in a loop. The row is rebuilt only when the width or the erase cell (BCE background) changes. Every path that blanks whole rows uses it: line-feed scroll, SU, SD, IL, DL, the alternate screen's spare rows and DECALN.
- **Measured:** `bench_vt_throughput` 4 MB, 120x40, interleaved runs against main: scroll_heavy 45 to 60 MB/s; a 38-row DECSTBM region scroll 76 to 136 MB/s. It composes with the blank-row flag (#1017): a clear that the flag cannot skip copies the cached row. On the combined code, quick-workload instructions per byte are scroll 111.8 (from 123.8), ascii 41.3 (from 44.4), sgr 109.0 (from 112.6), alt_screen 74.9; `bench/ir_ceilings.txt` is ratcheted to about +10% of those.
- **Known limitations:** Rows are still blanked at full width. Blanking only the written cells, and an O(1) screen rotate, are possible follow-ups and are not part of this change.
