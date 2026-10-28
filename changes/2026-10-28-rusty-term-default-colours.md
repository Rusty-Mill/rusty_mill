---
category: Fixed
changelog: rusty_term recolours text in the default colours when OSC 10/11 change them, and leaves colours a program chose alone.
---
## 2026-10-28 - rusty_term tracks default vs explicit colours

- **Fixed:** OSC 10/11 (and 110/111) no longer leave old colours baked into
  cells (#1163). A cell, pen or erase in the default foreground or background
  now carries `ATTR_FG_DEFAULT` / `ATTR_BG_DEFAULT`, and the style table fills
  in the current default when a cell is read, on screen, in history and on the
  stashed primary screen. A colour the child chose stays, even when it equals
  the old or new default. rt-caps.sh's OSC section no longer leaves a navy band.
- **Changed:** `ATTR_FG_DEFAULT`, `ATTR_BG_DEFAULT` and `ATTR_DEFAULT_COLORS`
  are new public constants; grid cells read back with them set in `flags`.
  They are outside `ATTR_MASK`. `Cell::blank()` is unchanged (explicit
  built-in colours).
- **Changed:** the RTSN snapshot is version 2: its header carries the default
  fg/bg. Version 1 still decodes, with every colour explicit as written.
- **Changed:** the RTCP checkpoint is version 4, negotiated by the new mux
  capability `CHECKPOINT_V4` (bit 16); `CHECKPOINT_V3` (bit 3) is reserved. v4
  keeps v3's layout, but its flags mean something new, so v3 checkpoints and
  v3-only mux peers are refused in both directions rather than restored with
  their default colours baked in.
