---
category: Changed
changelog: rusty_term's window copies each pane's frame under a brief lock and renders without it, so output no longer waits for compositing and present.
---
## 2026-10-24 - rusty_term: short frame lock

- **Changed:** `Grid::frame_view` copies what a frame draws into an owned `FrameView`: resolved cells, selection and search bits, bidi rows, the status line, scrollbar, command marks, placeholder cells and images. The window takes each pane's lock only for that copy, then composites and presents from the copies. Before, it held every visible pane's lock for the whole frame, present included, and the PTY reader waited it out.
- **Changed:** image pixels (`GridImage`, Kitty frames) are `Arc<[Option<u32>]>`, so a snapshot shares them instead of copying. An animated image's current frame is resolved at snapshot time.
- **Changed:** `bench_frame_time` measures the same split. At 120x40 and 120 Hz (local, 2026-10-24), lock hold is 0.08-0.09 ms p50 and 0.14-0.40 ms p99, against 0.99-2.12 ms p50 and 1.48-4.24 ms p99 on `main`. The worst single hold seen was 2.45 ms (one frame in two runs), against up to 6.08 ms on `main`. Parser MB/s while rendering now matches parsing alone; on `main` it ran 14-28% lower. Frame time is unchanged.
- **Known limitations:** wall-clock numbers are local. The lock is still a plain mutex with no fair handoff; no starvation was measured, and a lock-type change belongs with the tokio removal work. Every row is still copied each frame (backlog row 7).
