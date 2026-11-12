---
category: Added
changelog: The TUI's scripted-server lifecycle tests now also run on Windows, on our own console host.
---
## 2026-11-12 - TUI lifecycle tests on Windows

- **Added:** the 13 tests that run `rusty-mux` against a scripted server (terminal reset on every exit path, win32-input-mode stripped, the key log, split layouts, leases, last output, the v3 refusal) moved to `tui/tests/scripted.rs` and run on Linux and Windows (card #1423).
  - On Windows the client runs on our own console host (`rusty_conhost::OwnPty`), which keeps its VT bytes, so the same byte checks hold (pack D209, D211).
  - The shared harness is `tui/tests/common/mod.rs`. `lifecycle.rs` keeps the `--ssh` and reconnect tests, Linux only, because `--ssh` is Unix only.
