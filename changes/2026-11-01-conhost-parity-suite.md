---
category: Added
changelog: rusty_conhost gains a parity suite running every console API under ConPTY and our host, and fixes the window size, legacy attributes, colour table and font size it found differing.
---
## 2026-11-01 - Own console host parity suite

- **Added:** the console host's parity suite, the first Phase B criterion (design §9.2). `conhost_probe` gains a probe per console API, each a real client calling through kernelbase, and `own_pty`'s `parity_suite_agrees_with_conpty` runs about 100 of them under ConPTY and under our host and compares the answers. A difference must be a documented divergence (§5.10) or a tracked gap with an issue; a gap that starts to agree fails the suite until it is taken off the list. Tracked gaps: FillConsoleOutput and ScrollConsoleScreenBuffer (#1330), the cell read/write and buffer and window size calls (#1333), WriteConsoleInput (#1335), text past the margin with wrap off (#1336) and single writes of 64 KiB or more (#1337).
- **Fixed:** GetConsoleScreenBufferInfo on our host reported a window one cell narrower and shorter than the buffer. Legacy attributes now follow conhost: bold brightens a default or 16-colour foreground, SGR 4 no longer reads as `COMMON_LVB_UNDERSCORE` (conhost's bottom gridline), overline reads as the top gridline, and gridlines set by SetConsoleTextAttribute read back until a rendition reset (SGR 0, DECSTR, RIS), as conhost's `SetDefaultRenditionAttributes` clears them. The colour table starts as conhost's, and GetCurrentConsoleFont reports ConPTY's 0×16 cell.
- **Changed:** platform-windows admits the console client calls the probes make, as one block in `win32_surface` (no new dependency or feature; design D2 approval asked on the PR).
- **Known limitations:** an OSC 104 palette reset returns the host's model to xterm's colours, not conhost's.
