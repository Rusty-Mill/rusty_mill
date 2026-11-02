---
category: Added
changelog: rusty_mux terminal control: `terminal.size`, `terminal.reset` and `terminal.set-theme` block methods for ctl and MCP (C40).
---
## 2026-11-02 - rusty_mux terminal size, reset and theme (C40)

- **Added:** `terminal.size` reads a block's size as `COLSxROWS`, such as `80x24`.
- **Added:** `terminal.reset` resets a block's terminal by feeding it RIS, whose ESC also ends any sequence its program left unfinished (an OSC, DCS, APC or PM string included): screen, scrollback, modes and colours go back to how they started. It needs the lease.
- **Added:** `terminal.set-theme` sets the default foreground, background and cursor colours and palette entries (`--palette N=COLOUR`), each `#RRGGBB` or `default`, as the program could with OSC 10, 11, 12 and 4. Programs that ask see the colours, and clients draw with them. It needs the lease, checks every colour before anything is sent, and is refused while the program's output has stopped inside an escape sequence.
- **Added:** `Features::TERMINAL_CONTROL` (bit 26). ctl and MCP refuse the three methods before sending anything to a server without it. The MCP tools for reset and theme attach and take the lease first, and `terminal.reset` is marked destructive.
- **Added:** `rusty_term_core::AnsiParser::ground`, whether the parser is between sequences.
- **Known limitations:** reset and theme are bytes in the block's output, so they reach every attached client and, when the attach client passes a block through, the person's own terminal; colours set that way stay in that terminal after detaching, as a program's would. A theme lasts until the program or a reset changes it. A program that asked for mode 2031 reports is not told when the theme turns the background from dark to light. Tests run on Linux; Windows is checked by CI.
