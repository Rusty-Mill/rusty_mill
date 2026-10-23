---
category: Changed
changelog: rusty_term's windowed renderers resolve the fold-aware viewport once per frame, so command fold blocks no longer slow every frame.
---
## 2026-10-23 - rusty_term: view-row map once per frame

- **Changed:** `Grid::view` resolves every viewport row (live row, scrollback line or fold summary) in one walk over the fold blocks, and the CPU and GPU renderers read cells, selection, search highlights, bidi rows and image placeholders from it. Before, each per-cell lookup walked up to 1,024 fold blocks, several times per cell. The per-row `Grid` methods keep their behaviour and share the same resolution code.
- **Changed:** in `bench_frame_time` (120x40, local, 2026-10-23), 1,024 fold blocks now cost a frame 1.00-1.05 ms p50, against 0.99-1.36 ms with none. On `main` the same host measured 8.26-8.67 ms live and 12.32-13.09 ms scrolled back.
- **Known limitations:** wall-clock numbers are local. The grid lock is still held across the whole frame (backlog row 8), and every row is still rebuilt each frame (row 7).
