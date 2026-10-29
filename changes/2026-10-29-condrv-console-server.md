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
- **Admissions:** windows-sys `Win32_System_LibraryLoader` in
  platform-windows' own feature list; `LoadLibraryExW`, `GetProcAddress`,
  events, overlapped results and `CancelIoEx` in `win32_surface`; a second
  use of the already-admitted `NtCreateFile` (no new ntdll export).
