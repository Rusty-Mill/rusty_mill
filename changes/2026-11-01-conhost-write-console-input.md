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
- **Known limitations:** as in conhost, written records are not held to
  the 4,096-record cap that terminal input keeps, so a client that writes
  input and never reads it grows the queue (owner decision).
