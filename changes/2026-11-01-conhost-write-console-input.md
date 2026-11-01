---
category: Added
changelog: Our Windows console host serves WriteConsoleInput, so programs that inject keys into their own console work on it (#1335).
---
## 2026-11-01 - rusty_conhost: WriteConsoleInput

- **Added:** WriteConsoleInputW and WriteConsoleInputA queue the client's
  records as conhost does, and waiting reads (GetConsoleInput, ReadConsole,
  cooked or raw) are served at once. `A` records are converted from the
  input code page, a DBCS lead byte waiting for the next write. A VT-input
  client gets written keys as VT. A single appended record coalesces as in
  conhost (key repeats, mouse moves, buffer sizes). Mouse and menu records
  are kept as written.
- **Changed:** a write that would take the input queue past 4,096 records
  fails with `STATUS_NO_MEMORY` and queues nothing (divergence V16;
  conhost has no cap).
