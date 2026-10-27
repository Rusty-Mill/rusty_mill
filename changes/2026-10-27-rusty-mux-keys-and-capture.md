---
category: Added
changelog: rusty_mux named keys and capture options: `terminal.send-keys` presses keys such as `ctrl+c`, `up` or `f5` in a block, and `terminal.format` can trim blank lines, unwrap wrapped rows and keep colors as SGR.
---
## 2026-10-27 - rusty_mux named keys and capture options (C32)

- **Added:** the `terminal.send-keys` block method (`rusty-mux-ctl terminal.send-keys --block … --key ctrl+c --key enter`, and an MCP tool that takes the lease first, as `input` does). Each `key` is a character or a named key (`enter`, `tab`, `space`, `backspace`, `escape`, `up`, `down`, `left`, `right`, `home`, `end`, `page-up`, `page-down`, `insert`, `delete`, `f1` to `f12`) after any of `ctrl+`, `alt+` and `shift+`. The server encodes them as xterm does, so cursor keys follow the program's application cursor mode (DECCKM), and refuses a name it does not know or a modifier a key has no encoding with, before writing anything.
- **Added:** `terminal.format` options `trim` (leave out blank lines at either end), `unwrap` (join rows the program's text wrapped onto, keeping their blanks, without the blank a wide glyph left) and `vt` (colors and attributes as SGR sequences, each line starting from the default style and ending in it), behind the new `Features::CAPTURE` (bit 13). `rusty-mux-ctl` takes them as bare flags (`--trim`), MCP and actions as booleans.
- **Added:** a `Flag` parameter type in the request catalog, carried as one byte in a block call's arguments; `rusty_term_core` exports `Cell` and `Grid::line_cells` so the server can read a row's styles and wrap bit.
- **Known limitations:** `vt` writes truecolor only (the grid keeps resolved colors, not palette indexes), so a cell explicitly set to the default color reads as default; it carries no OSC 8 links. HTML capture is not built.
