---
category: Changed
changelog: rusty_mux's server event-stream tests (status, pwd/title, clipboard, replay) now also run on Windows.
---
## 2026-11-10 - rusty_mux server event tests on Windows

- **Changed:** `server/tests/events.rs` runs on Windows as well as Linux. `cmd.exe` prints each OSC with an ESC taken from a `prompt $E` child shell, ended by ST; checked on both console hosts (card #282, item 1).
- **Known limitations:** bell/progress/notify with an exit code, foreground-process changes, the status-query round trip (`stty`/`dd`) and the `sh` launch test stay Linux-only.
