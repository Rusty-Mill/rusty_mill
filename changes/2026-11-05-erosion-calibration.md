---
category: Added
changelog: Erosion calibration report over 1,182 merged PRs, with the scripts that reproduce it, proposing limits for Nano.
---
## 2026-11-05 - Erosion gate: calibration

- **Added:** `docs/projects/let-em-loose/erosion-calibration.md` (card #704 slice 5, D110). It measures every merged non-bot PR's merge commit against its first parent and reports p50 to p99 of each delta, the share of PRs each limit pair would fail, unmeasured code by extension and the gaming signal. It covers the strict set (OPEN_QUESTIONS Q36) and the wide set. The card's p90 rule gives 0 / 0 bp, and the report recommends 1 / 1 bp; Nano sets the limits (D42).
- **Added:** `crates/tools/rusty_erosion/calibrate/` (`prs.sh`, `measure.sh`, `summarize.py`), which reproduce the report from a full-history clone.
- **Docs:** D110 (slices 4–5 build defaults) and OPEN_QUESTIONS Q36 (which PRs count as human).
