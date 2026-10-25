---
category: Changed
changelog: rusty_term_core checkpoints are RTCP v3, run-encoded lines that bring a 200x50 attach with 10,000 history rows from 44.3 MB to 0.64 MB; rusty_mux negotiates `CHECKPOINT_V3` (bit 3) and no longer offers v2.
---
## 2026-10-25 - Compact RTCP v3 checkpoints

- **Changed:** `rusty_term_core::checkpoint` writes RTCP v3. A line is a varint cell count (at most 4096, checked before allocating) and runs of one style, each a repeat of one char or a literal string of UTF-8 chars, with styles defined once per checkpoint and then referenced by index. The rest of the schema is unchanged. On 12,000 lines of coloured text a 200x50 checkpoint drops from 44.32 MB to 0.64 MB (the RTSN snapshot of the same grid is 0.65 MB), encode from 67 to 19 ms and decode from 45 to 25 ms. That encode runs under the mux server's state borrow, so this also shortens the stall an attach causes for other blocks. `Cell` now derives `Hash`.
- **Changed:** rusty_mux `Features::CHECKPOINT_V3` is bit 3 and is the only checkpoint feature the server, the TUI and the tests offer. Bit 2 (`CHECKPOINT_V2`) is now reserved: no released client spoke it. `docs/projects/mux/REPLAY.md` documents the v3 format and the measurements.
- **Known limitations:** an attach still sends a full checkpoint; incremental deltas are not built. Encoding a 200x50 grid with full history still takes about 19 ms under the server's state borrow, short of the 15 ms aim; the per-cell run scan and the existing grid validation are most of it.
