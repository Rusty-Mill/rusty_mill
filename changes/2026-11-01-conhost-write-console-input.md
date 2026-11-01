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
- **Known limitations:** a write is never refused, so a client that
  writes input and never reads it grows the queue past 4,096 records
  (owner decision). Terminal input arriving at that size drops the oldest
  record first, which may be one the client wrote; conhost drops neither.
