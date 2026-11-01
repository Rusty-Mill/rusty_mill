---
category: Added
changelog: New rusty_conhost crate, our own Windows console host (output only): a sans-I/O console over ConDrv with a pty-shaped OwnPty front end.
---
## 2026-10-29 - rusty_conhost: our own console host, output only

- **Added:** `rusty_conhost`, host PR 2 of the console-host plan
  (`docs/projects/term/conhost/design.md` §10). A sans-I/O `Console`
  serves ConDrv's packets and turns console calls into VT, behind one
  engine thread with the §7 input and control lanes and the §7.4 output
  caps. `OwnPty`/`OwnPtyMaster`/`ExitWaiter` give it a pty-shaped front
  end on Windows.
- **Added:** connection and object handling, share modes, GetConsoleMode
  and SetConsoleMode, WriteConsole, RAW_WRITE, Get/SetConsoleTitle,
  Get/SetConsoleCP (65001 only) and GetConsoleLangId, following
  conhost's rules and status codes (microsoft/terminal, MIT).
- **Added:** text fixtures replayed on Linux, a sanitised session
  recorder (`RUSTY_CONHOST_RECORD`), diagnostics on stderr behind
  `RUSTY_CONHOST_LOG`, and a probe example for parity runs against
  ConPTY.
- **Added:** rusty_term parses a `[console] host = "conpty" | "own"`
  key. It is not acted on or documented yet.
- **Known limitations:** terminal input is discarded, a resize only
  records the size, and the console closes as soon as the terminal hangs
  up or the child exits. Input, the screen model, CTRL_CLOSE and the
  rusty_term backend follow in later PRs.
