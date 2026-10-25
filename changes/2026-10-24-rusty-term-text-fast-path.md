---
category: Changed
changelog: rusty_term writes runs of non-ASCII text in one step instead of one scalar at a time, cutting unicode_heavy's instructions per byte by 31%.
---
## 2026-10-24 - rusty_term: non-ASCII text fast path

- **Changed:** in the ground state the parser takes the leading run of well-formed non-ASCII UTF-8 with std `utf8_chunks` (no new dependency) and hands it to the grid in one call. Each call examines at most 64 bytes, so a stream of malformed high bytes still parses in linear time. The new `Grid::put_text` fills the glyphs that fit on the row straight into its cells and repairs a wide glyph only where the run starts or ends. A wrap, a zero-width scalar (combining mark, C1 control), insert mode, DECSLRM margins and mode 2027 still take `put_char`. Malformed and chunk-split sequences still go through the byte-wise decoder, so replacement-character handling is unchanged.
- **Changed:** `put_char` looks up a scalar's width once instead of up to three times.
- **Changed:** instructions per byte: unicode_heavy 168 to 117; the other workloads move by 1.5 or less (cursor_thrash 85 to 87). The unicode_heavy ceiling in `bench/ir_ceilings.txt` drops from 184 to 128.
- **Tested:** one test compares the fast path with `put_char` one scalar at a time, over every start column and run length, across wide glyphs, DECAWM off, insert mode, margins, a scroll region and mode 2027. A second test compares whole and every two-chunk split of valid, truncated, overlong, surrogate, out-of-range and C1 input, and a sequence cut by the 64-byte window, with the byte-wise decoder. A third feeds 256 KiB of malformed high bytes in one call, which a per-byte rescan would turn into ~10^10 steps. A fourth clears the screen, writes a run past column zero, and clears and reuses the row twice, checking glyphs and row stamps after each step.
- **Known limitations:** wall-clock gains are smaller than the instruction drop: unicode_heavy went from 47.1 to 50.5 MB/s (median of 16 runs, 4 MB, a shared 4 vCPU VM with about ±10% noise). Combining marks still cost a `String` allocation and a linear table search each; that is the next change.
