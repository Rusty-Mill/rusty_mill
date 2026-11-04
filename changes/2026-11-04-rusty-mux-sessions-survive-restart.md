---
category: Added
changelog: rusty_mux sessions, layouts and screens survive a server restart (C45), saved to an owner-only state directory.
---
## 2026-11-04 - rusty_mux sessions survive a server restart (C45)

- **Added:** `rusty-mux-server` saves its sessions, windows, layouts, block names and each block's screen and scrollback every 5 s when something changed, and in full on a clean stop. The store is `--state DIR`, by default `$XDG_STATE_HOME/rusty_mux/<socket name>` or `%LOCALAPPDATA%\rusty_mux\<socket name>`, and is owner-only. `--no-state` turns it off. The next server on that directory restores everything with the same ids. Each block gets a fresh shell in its last working directory, below its saved screen and a one-line "restored" marker.
- **Added:** `rusty_mux_core::Mux::save` and `Mux::restore` (`Saved`, `SavedSession`, `SavedWindow`, `SavedBlock`, `RestoreError`), plus `Config::state_dir` in `rusty_mux_server`, which is a public API change.
- **Known limitations:** programs that were running are not restarted. A block started with its own command comes back as a shell, and its marker names the command. Screen text is stored unencrypted until encryption at rest lands. On Linux the server has no clean stop yet, so a signal or crash can lose up to the last 5 s. A store that cannot be read is moved aside to `<dir>.bad-<seconds>`, and the server starts empty.
