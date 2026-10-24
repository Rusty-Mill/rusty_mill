---
category: Changed
changelog: rusty_term blanks scrolled-in and erased rows by copying one cached blank row instead of a per-cell fill, about 30% faster on scroll-heavy output.
---
## 2026-10-24 - rusty_term cached blank row

- **Changed:** `Grid` keeps one prebuilt row of blank cells, and `Line::clear` copies it in a single `memcpy` instead of storing each cell in a loop. The row is rebuilt only when the width or the erase cell (BCE background) changes. Every path that blanks whole rows uses it: line-feed scroll, SU, SD, IL, DL, the alternate screen's spare rows and DECALN.
- **Measured:** `bench_vt_throughput` 4 MB, 120x40, interleaved runs against main: scroll_heavy 45 to 60 MB/s; a 38-row DECSTBM region scroll 76 to 136 MB/s. Instructions per byte on the quick workloads (`ir_gate.py`), before and after on main 0bd20ec4: scroll 136.7 to 104.7, alt_screen 735.0 to 522.7, ascii 47.8 to 39.6, sgr 116.6 to 106.9. `bench/ir_ceilings.txt` is ratcheted to about +10% of the counts measured after merging the changed-row stamps (#993).
- **Known limitations:** Rows are still blanked at full width. Blanking only the written cells, and an O(1) screen rotate, are possible follow-ups and are not part of this change.
