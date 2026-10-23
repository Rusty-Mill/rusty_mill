---
category: Changed
changelog: rusty_term scrolls by rotating rows instead of moving cells, and reuses the scrollback line it evicts as the new bottom row; scroll-heavy output is about 3x faster with flat memory.
---
## 2026-10-22 - rusty_term: row-rotating scroll with recycled scrollback lines

- **Changed:** the screen is now one `Line` per row, the same type scrollback holds: cells, soft-wrap bit and line size (DECDWL/DECDHL). A scroll rotates rows; it no longer copies every cell of the region. The line leaving the top moves whole into scrollback. Once scrollback is full, the line it evicts is blanked and becomes the new bottom row, so steady-state scrolling allocates nothing. IL, DL, SU and SD rotate rows the same way, and the separate per-row wrap and size tables are gone.
- **Changed:** scroll_heavy went from about 3.3 to 10.5 MB/s locally (1003 to 259 instructions per byte), and memmove fell from 75% to under 5% of its instructions. ascii_throughput went from about 11 to 24 MB/s and sgr_churn from 431 to 208 instructions per byte. The instruction-count ceilings in `bench/ir_ceilings.txt` are lowered to match.
- **Changed:** memory is flat: 47.8 MiB for 10,000 scrollback lines at 200x50 (24.4 B/cell before and after), and peak RSS (VmHWM) is unchanged at about 52 MiB. With full scrollback (30,000 lines written), RSS stays at 48 MiB and the fill takes 138 ms instead of 339 ms.
- **Changed:** the alternate screen keeps its rows after an app leaves it and reuses them on the next entry. This costs one extra screen of memory after the first alternate-screen use and avoids a per-row allocation on every switch.
- **Changed (API):** `Grid::cells` is no longer public. Read a row with `Grid::row(y)`; `Grid::row_mut(y)` gives mutable access.
- **Known limitations:** the scrollback is still one heap allocation per line. The print path (`put_char`, `set_cell`) and the parser now dominate; they are separate backlog items.
