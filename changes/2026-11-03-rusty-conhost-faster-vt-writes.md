---
category: Changed
changelog: rusty_conhost passes client VT through 1.2-4x faster by copying plain runs and unwatched CSIs whole and no longer allocating per sequence.
---
## 2026-11-03 - rusty_conhost passes client VT through faster

- **Changed:** The own console host's VT write path no longer works byte by byte on text it does not change:
  - Runs of text up to the next ESC or LF are copied in one go, and so is any complete CSI that the host does not watch.
  - The watched-sequence check reads parameters only for DSR and DECRST, and does so without allocating.
  - A slice with nothing watched in it is forwarded without being matched again.
  - The transform buffer is reused between writes.

  On the 4 MB workloads, at 80x24 with 64 KiB writes, the host's screen path runs 1.2–4x faster (sgr_alternate 113 → 37 ms, cursor_thrash 116 → 40 ms, ascii 44 → 11 ms on Linux). Output is unchanged: it was byte-identical to the previous code across 6160 differential runs that covered every workload, randomly generated VT, all output modes and small `room` limits.
- **Known limitations:** unicode_2027 gains least (154 → 129 ms), because its time is spent in rusty_term_core's grid rather than in the host.
