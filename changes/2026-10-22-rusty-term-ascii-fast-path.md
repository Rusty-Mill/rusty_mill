---
category: Changed
changelog: rusty_term writes runs of printable ASCII in one step instead of one character at a time, cutting the parser and grid's instructions per byte by up to 4x.
---
## 2026-10-22 - rusty_term: printable-ASCII fast path

- **Changed:** in the ground state with ASCII in GL, the parser finds the run of printable ASCII eight bytes at a time (a std-only word scan, no new dependency) and hands it to the grid in one call. The grid fills the part that fits on the row straight into the row's cells and repairs a wide glyph only where the run starts or ends. Wrapping, insert mode and DECSLRM margins still take the one-character path. A test compares the two paths over every start column and run length, across wide glyphs, DECAWM off, insert mode, margins and a scroll region.
- **Changed:** instructions per byte: ascii_throughput 193 to 48, scroll_heavy 259 to 137, sgr_churn 208 to 117, cursor_thrash 127 to 85, alt_screen_flicker 848 to 735; unicode_heavy is unchanged (168). The ceilings in `bench/ir_ceilings.txt` are lowered to match.
- **Known limitations:** wall-clock MB/s rose less than the instruction counts fell (ascii_throughput about 28 to 36 MB/s locally). Each benchmark iteration builds a fresh grid, and about half the run time is now the kernel faulting in new scrollback memory (24 bytes per cell). With glibc told to keep freed memory, the same binary runs ascii_throughput at over 100 MB/s and scroll_heavy at about 40 MB/s.
