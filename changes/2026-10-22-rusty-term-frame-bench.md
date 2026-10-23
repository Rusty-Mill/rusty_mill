---
category: Added
changelog: rusty_term has a frame-time benchmark for the windowed front-end's CPU frame path, reporting frame time and grid-lock hold time while output streams in.
---
## 2026-10-22 - rusty_term: frame-time benchmark

- **Added:** `bench_frame_time` (feature `gui`). A run that cannot measure (no font, a renderer that fails to start or panics, no samples) exits non-zero instead of reporting zeros. Percentiles are nearest rank. It paints the window's real CPU frame path offscreen. In streaming mode, a parser thread feeds each workload into a shared grid in 4 KiB chunks while a render thread paints at 60 Hz, holding the grid lock for the frame as the window does. It reports frame and lock-hold percentiles, the render thread's wait for the lock, and parser MB/s with and without rendering. A static mode times frames with 0 and 1,024 command fold blocks, at the live view and scrolled back.
- **Changed:** the CPU renderer's frame painting is split from presentation (`render::composite`), so the benchmark runs the same code as the window. Behaviour is unchanged.
- **Changed:** baseline at 120x40, local, over two hosts (2026-10-22 and 2026-10-23): the lock is held 0.95-2.63 ms per frame at p50, 2.1-7.3 ms at p99 and up to 12.3 ms. With the same text, 1,024 fold blocks make a frame 8.6x slower at the live view (1.02 to 8.79 ms p50) and 13x slower scrolled back (1.02 to 13.47 ms). These are the measurements the GUI frame-path work starts from.
- **Known limitations:** wall-clock only, so it is a local tool and not a CI gate. It measures the CPU renderer, not the GPU one.
