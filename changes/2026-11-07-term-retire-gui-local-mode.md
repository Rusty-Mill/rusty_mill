---
category: Changed
changelog: rusty_term: `--gui` now opens rusty-mux-gui, a rusty_mux client; the window's local mode is retired. rusty-mux-gui gains --cwd, --profile, --config, --setup and `-- command`.
---
## 2026-11-07 - rusty_term's window becomes a rusty_mux client

- **Changed:** `rusty_term --gui` runs `rusty-mux-gui` (beside `rusty_term`, else on `PATH`), which starts a server on the default socket when none answers (C69). It passes on `--gpu`, `--setup`, `--config`, `--profile`, `--cwd`/`--starting-directory` and the command after `--`, `-e` or `--command`.
- **Added:** `rusty-mux-gui` takes `--config`, `--profile`, `--cwd`, `--setup` and `-- <program> [args...]`. A new session's first block runs the command, else the profile's or config's `shell`, in `--cwd`, else the profile's or config's `cwd`; splits and new windows run the same shell. First-run setup opens in the mux window too.
- **Removed (for now):** with `--gui`, `--title`, `--maximized`, `--fullscreen`, `--opacity`, `--session` and `--single-instance` are ignored with a note, and the window `ctl` commands (`new-window`, `quake`, `focus-tab`) have no mux equivalent yet. The local-mode code stays until its deletion gets its own go.
