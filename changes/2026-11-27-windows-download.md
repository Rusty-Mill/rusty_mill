---
category: Added
changelog: terminal.download now works against a Windows rusty-mux-server.
---
## 2026-11-27 - File download from a Windows server

- **Added:** a Windows `rusty-mux-server` answers `terminal.download` (C83, card #1586). The file is judged on the open handle (final path, link check, file index), so a swapped link or a hard link to a protected file is refused as on Linux. Adds `rusty_win32` to the server on Windows; the only `unsafe` is two single-call helpers in `download_win.rs`.
- **Known limitations:** macOS still builds the stub server, so nothing is served there. The Windows read has unit tests but no live-server round trip in CI.
