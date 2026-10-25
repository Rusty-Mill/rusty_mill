---
category: Changed
changelog: rusty_term skips re-blanking rows that are already blank, making alternate-screen switches about 20x faster.
---
## 2026-10-24 - rusty_term skips clears of already-blank rows

- **Changed:** each grid row remembers that it is uniformly blank in a given fill, and a clear or `ED 2` with the same fill and width skips it. An alt-screen switch used to blank the whole reused buffer on entry and then again on the app's `CSI 2J`, cell by cell; together those were 95% of the alt_screen_flicker instructions. `Line` moves to `grid/line.rs` with its cells private, so every write goes through `cells_mut()`, which drops the flag, and a debug assertion checks every skipped row.
- **Measured:** `bench_vt_throughput` 4 MB, 120x40: alt_screen_flicker 4.9 to 107 MB/s, other workloads unchanged within noise. Instructions per byte on the quick workloads (`ir_gate.py`): alt 735.0 to 73.4, scroll 136.7 to 123.8, ascii 47.8 to 44.4, sgr 116.6 to 112.6; `bench/ir_ceilings.txt` is ratcheted. `bench_frame_time` on alt_screen_flicker: render-wait p99 1.1 s to 58 ms.
- **Known limitations:** the flag is per row, so one written cell makes the next clear rewrite the whole row. Full screens still pay a full per-cell clear (Ghostty is about 1.5x faster there); a cheaper bulk clear belongs to the deferred compact-cell work.
