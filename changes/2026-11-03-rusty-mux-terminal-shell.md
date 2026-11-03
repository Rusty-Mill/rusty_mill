---
category: Added
changelog: rusty_mux blocks answer `terminal.shell`, which reports the block's shell, whether it is idle or busy, and its OSC 133 prompt phase and last exit code.
---
## 2026-11-03 - rusty_mux `terminal.shell` (C71 part 2)

- **Added:** `rusty-mux-ctl terminal.shell --block B` prints `program`, `activity` (`idle`, `busy` or `unknown`, from the process table), `phase` (`prompt`, `running` or `finished`, from the shell's OSC 133 marks) and `exit` (its last command's exit code) as `NAME<TAB>VALUE` lines; the OSC 133 lines read `unknown` until the shell sends a mark.
- **Changed:** `rusty_term_core` tracks the shell's OSC 133/633 phase and last exit code in every build (`Grid::shell_phase`, `Grid::last_command_exit`), not only under `gui` or `l13`.
- **Protocol:** a block call behind `Features::TERMINAL_SHELL` (bit 32). MCP exposes it as a read-only tool.
