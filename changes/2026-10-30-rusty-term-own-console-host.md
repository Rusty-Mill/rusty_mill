---
category: Added
changelog: rusty_term can run Windows shells on our own console host, rusty_conhost, with `[console] host = "own"`; ConPTY stays the default.
---
## 2026-10-30 - rusty_term: run shells on our own console host

- **Added:** host PR 5 of the console-host plan
  (`docs/projects/term/conhost/design.md` §4.5, §10), the milestone. On
  Windows, `[console] host = "own"` makes rusty_term serve the shell's
  console with rusty_conhost instead of ConPTY, in the terminal and in the
  window (`--gui`). The key is now documented in the README. ConPTY is
  still the default.
- **Added:** if the own host can't start, the error is shown. There is no
  silent fallback to ConPTY.
- **Known limitations:** with `own`, the `shell` string is split into a
  program and arguments by `CreateProcessW`'s rules, and each argument is
  quoted again. Programs that read their command line raw, cmd.exe above
  all, can see different quoting from what ConPTY would give them. The web bridge
  always uses ConPTY. The console host does not yet serve
  FillConsoleOutput, ScrollConsoleScreenBuffer, the
  Read/WriteConsoleOutput family or code pages other than 65001 (§10 PRs 6,
  7 and 9).
