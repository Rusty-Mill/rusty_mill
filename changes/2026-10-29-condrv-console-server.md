---
category: Added
changelog: platform-windows can serve the Windows console driver directly (`condrv`), the first step of our own console host (#1222).
---
## 2026-10-29 - platform-windows: ConDrv console server (#1222, host PR 1)

- **Added:** `platform_windows::condrv`: open a console server, spawn a
  child attached to it, and serve its packets with one overlapped READ_IO,
  a `Ready` snapshot wait with an absolute deadline, cancellation that
  drains before freeing, and Ctrl delivery through `user32!ConsoleControl`
  resolved at runtime. Contract: `docs/projects/term/conhost/design.md`
  §11.2.1. Windows tests in `tests/condrv.rs` answer real clients (cmd,
  ping) with no conhost involved.
- **Corrected:** spawning no longer changes the embedding process's
  process-wide Ctrl+C policy. Isolated helper tests cover successful and
  failed spawns from a Ctrl+C-ignoring parent, while retaining delivery
  coverage from an enabled parent. A failure after process creation
  terminates the child and returns once it has exited; if that can't be
  confirmed within 5 s, the cleanup error is returned and the child's
  handles stay open on a background thread until it exits.
- **Admissions:** windows-sys `Win32_System_LibraryLoader` in
  platform-windows' own feature list; `LoadLibraryExW`, `GetProcAddress`,
  events, overlapped results and `CancelIoEx` in `win32_surface`; a second
  use of the already-admitted `NtCreateFile` (no new ntdll export).
- **Validation scope:** the verified D4 run at `208a5b5c` used Windows
  Server 2025 (10.0.26100), image `windows-2025-vs2026`, version
  `20261021.278.1`; its Windows tests and required gates passed. Beast and
  WOW64 results remain author-reported. An older-machine run and a
  conclusive Ctrl+Break probe are still outstanding.
