---
category: Added
changelog: Research note ranking rusty_term performance work against Ghostty, Alacritty, WezTerm and kitty, with a measured baseline against libghostty-vt.
---
## 2026-10-21 - rusty_term performance backlog

- **Added:** `docs/research/RUSTY-TERM-PERF-BACKLOG.md`: 19-row technique backlog (who does it, rusty_term today, win, effort, risk, priority), a top-5 order with the benchmark that proves each, and where the headless split and snapshot serializer fit.
- **Added:** baseline numbers. rusty_term core is 1.6-4.5x slower than libghostty-vt with 10k lines of scrollback, 92-97% of instructions are the whole-screen memmove in `Grid::scroll_up`, memory is 24.4 vs 8.3 B/cell, resize is 6-13x slower, and there is no snapshot serializer. Base glyphs match libghostty on the six synthetic workloads.
- **Added:** `docs/research/rusty-term-perf/{rtbench,ghbench}`: standalone harnesses (not workspace members) that rerun the memory, resize, snapshot and fidelity numbers. libghostty-vt is a comparison oracle only.
- **Known limitations:** Linux only; vtebench, vttest, captured sessions, GPU frame time and Windows were not run. No engine changes.
