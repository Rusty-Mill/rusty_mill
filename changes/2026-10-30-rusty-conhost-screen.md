---
category: Added
changelog: rusty_conhost keeps its own screen over rusty_term's Grid, so programs that read the cursor, colours and buffer size, such as pwsh with PSReadLine and edit.exe, can run on our own console host.
---
## 2026-10-30 - rusty_conhost: screen over Grid

- **Added:** host PR 4 of the console-host plan
  (`docs/projects/term/conhost/design.md` §10). Every byte the host
  sends the terminal also feeds an `AnsiParser` and a `Grid` with no
  scrollback. The output modes apply as in §5.4. DSR 5 and CPR queries
  are answered from that Grid and kept from the terminal (§5.6).
- **Added:** GetConsoleCursorInfo and SetConsoleCursorInfo,
  GetConsoleScreenBufferInfo, SetConsoleCursorPosition,
  SetConsoleTextAttribute, GetCurrentConsoleFont, and
  GetConsoleWindow, which returns NULL. Resizing reflows the host's
  Grid and queues a WINDOW_BUFFER_SIZE_EVENT under WINDOW_INPUT.
- **Added (public API):** `rusty_term_core::{ColorRef, CellColors}`,
  `AnsiParser::pen_colors`, `AnsiParser::palette` and
  `Grid::cell_colors` and `Grid::origin_region` (design §4.3,
  §11.2.3). The wire formats are unchanged.
- **Added:** a client's DECRST that turns off win32-input-mode or
  focus reports among other modes is caught too, and what the host adds
  to client output (modes after RIS, CR LF on a legacy wrap) counts
  against the emission budget.
- **Fixed:** rusty_term_core's CPR and DECXCPR report a pending wrap
  as the last column, as xterm does, instead of one past it.
- **Known limitations:** after a resize, the host's screen and the
  terminal's can differ until a redraw (§5.7, V3). Regression tests pin
  this divergence. GetCurrentConsoleFont reports a fixed 8×16 Consolas
  cell.
