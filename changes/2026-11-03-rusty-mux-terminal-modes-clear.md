---
category: Added
changelog: rusty_mux blocks answer `terminal.modes`, the terminal modes their program set, and `terminal.clear`, which clears the screen and scrollback.
---
## 2026-11-03 - rusty_mux `terminal.modes` and `terminal.clear` (C71 part 1)

- **Added:** `rusty-mux-ctl terminal.modes --block B` prints one `NAME<TAB>VALUE` line per mode the block's program set: application cursor keys and keypad, bracketed paste, focus reporting, mouse tracking and encoding, the alternate screen, alternate scroll, insert mode, cursor visibility and blink, colour-scheme reports, win32-input and the Kitty keyboard flags.
- **Added:** `terminal.clear` clears the screen and scrollback for the server and every attached client, as `clear` does, keeping modes, colours and the title. It needs the lease and waits until the program's output is not inside an escape sequence.
- **Protocol:** both are block calls behind `Features::TERMINAL_MORE` (bit 31). MCP exposes them as tools; `terminal.clear` is marked destructive.
