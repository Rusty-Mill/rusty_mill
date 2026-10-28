---
category: Fixed
changelog: rusty_term's window draws dim, reverse, hidden and overlined text, and SGR 21 gives a double underline.
---
## 2026-10-28 - rusty_term window draws dim, reverse, hidden and overline

- **Fixed:** the window (CPU and GPU renderers) now draws SGR 7 with fg and
  bg swapped, SGR 2 with the foreground faded a third of the way toward the
  background, and SGR 8 as background only (no glyph, underline, strike or
  overline). The TUI and the mux already re-emitted these; only the window
  dropped them.
- **Added:** overline. SGR 53 sets it and SGR 55 or 0 clears it; the window
  draws a line along the top of the cell, the TUI, the mux's
  `terminal.format --vt` and DECRQSS report it as `53`, and HTML copy uses
  `text-decoration: overline`.
- **Fixed:** SGR 21 is a double underline (as `4:2`), as current
  xterm-compatible terminals read it; SGR 24 clears it.
- **Fixed:** regression tests pin that straight and double underlines take
  the SGR 58 colour, including `58;5;n`, in both window renderers.
- **Changed:** `Cell::flags`, `Pen::attrs` and the `ATTR_*` constants are
  `u32` instead of `u16`, to make room for `ATTR_OVERLINE` (bit 16).
- **Changed:** the RTCP checkpoint is version 6, negotiated by the new mux
  capability `CHECKPOINT_V6` (bit 18); `CHECKPOINT_V5` (bit 17) is reserved.
  v6 writes cell, style and pen flags as 32 bits. v5 and older checkpoints and
  v5-only mux peers are refused in both directions.
- **Changed:** RTSN snapshots are version 3, whose style flags may use 32
  bits. Versions 1 and 2 still decode, their flags limited to 16 bits.
- **Known limitations:** on the GPU renderer an overline on a colour (emoji)
  glyph is drawn white.
