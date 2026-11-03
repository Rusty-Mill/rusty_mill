---
category: Fixed
changelog: rusty_term draws double-width and double-height lines and combining marks in the window, and mode 2027 gives VS16 emoji and keycaps two columns, joining clusters without allocating.
---
## 2026-10-28 - rusty_term double-size lines, combining marks and mode 2027 widths

- **Fixed:** the window (CPU and GPU renderers) draws `ESC # 6` (DECDWL)
  lines at twice the cell width and `ESC # 3` / `ESC # 4` (DECDHL) halves at
  twice the width and height, together showing one tall glyph. The CPU
  renderer puts a DECDHL line's underline on its bottom half and overline and
  strike on its top half; the GPU renderer samples half the glyph tile per
  row and splits decorations the same way.
- **Changed:** a double-width or double-height row holds half the columns:
  text wraps (or, with autowrap off, pins) at the middle, and setting the
  attribute moves a cursor in the right half to the last column shown. This
  matches the host terminals the TUI relays the attribute to.
- **Fixed:** the window draws a cell's combining marks (`e` + U+0301 shows
  as é), centered over the base glyph. Joiners and variation selectors draw
  nothing.
- **Fixed:** with mode 2027 set, a VS16 that makes an emoji presentation
  sequence (`❤️`, `1️⃣`) widens the glyph to two columns, wrapping first when
  it does not fit; without autowrap it stays one column. Mode 2027 reset is
  unchanged (per code point).
- **Changed:** mode 2027 decides each cluster boundary from the base scalar
  and its interned suffix in place, with no allocation or re-segmentation per
  code point. A new `unicode_2027` bench workload is gated at 379
  instructions/byte (measured 344.8, from 500.1).
- **Known limitations:** mouse selection on a double-width row still maps
  clicks to single-width columns. The GPU renderer's double-size and mark
  paths are unit-tested but were not run against a real adapter.
