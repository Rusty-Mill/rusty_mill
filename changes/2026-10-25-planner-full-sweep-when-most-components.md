---
category: Changed
changelog: CI runs the full workspace sweep when a change reaches more than half of the workspace's components, instead of one job per component.
---
## 2026-10-25 - Full sweep when most components are affected

- **Changed:** the planner runs the full sweep when the affected crates span more than half of the workspace's CI components (`ci_components.py --full-if-most`), instead of one clippy and test lane per component. The full sweep tests every crate, so no selected crate goes untested.
- **Why:** each lane rebuilds the shared dependencies. #1066 (`rusty_tokio`) reached 28 of 45 components and ran 145 jobs: about 350 runner-minutes, 88 of them clippy. A full sweep is 58 jobs and about 270 runner-minutes, 25 of them clippy. With runners scarce, the job count also kept #1066 at the head of the merge queue for over 50 minutes. On current `main`, a `rusty_tokio` change (102 crates, 27 lanes) now takes the full sweep. A `rusty_term` core change (4 lanes) or a `rusty_tonic` change (3 lanes) stays scoped.
