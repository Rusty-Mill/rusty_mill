---
category: Fixed
changelog: rusty_term recolours text drawn in palette colours when OSC 4 changes them, saves the pen with DECSC, and keeps the status line in theme colours.
---
## 2026-10-28 - rusty_term palette recolouring, DECSC pen and theme status line

- **Fixed:** OSC 4, OSC 104, RIS and DECSTR now recolour text already drawn in
  a palette colour (SGR 30-37, 90-97, 40-47, 100-107 and `38/48;5;n`), on both
  screens and in history, and BCE erases made in one. A colour given as RGB
  stays, even when it equals the entry. The pen and saved pens follow too.
- **Fixed:** DECSC (`ESC 7`, `CSI s`) and DECSET 1049 save the pen, DECSCA
  protection and the charsets with the cursor, one save per screen, as xterm
  does; DECRC restores them, a saved default or palette colour taking its
  current value. DECRC with nothing saved resets them.
- **Fixed:** the status line drawn in the default colours (L13) follows OSC
  10/11 and theme changes instead of keeping the colours it was set in.
- **Changed:** the RTCP checkpoint is version 5, negotiated by the new mux
  capability `CHECKPOINT_V5` (bit 17); `CHECKPOINT_V4` (bit 16) is reserved.
  v5 adds palette tags to line styles, the pen and the erase colour, the DECSC
  state, and optional status-line colours. v3 and v4 checkpoints and v3/v4-only
  mux peers are refused in both directions. RTSN is unchanged.
