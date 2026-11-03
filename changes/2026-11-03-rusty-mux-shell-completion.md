---
category: Added
changelog: rusty_mux: `rusty-mux-ctl completion bash|zsh|fish|nu|powershell` prints a shell completion script generated from the catalog (C52).
---
## 2026-11-03 - rusty_mux shell completion

- **Added:** `rusty-mux-ctl completion SHELL` prints a completion script for bash, zsh, fish, Nushell or PowerShell. It completes commands, methods and their flags (after `+` too), method names after `help` and `api`, and paths after `--socket`, `--cwd` and `--dir`. Generated from the catalog, so new methods complete without changes.
- **Known limitations:** targets (block, window and session names) are not completed. Nothing is installed; load the script from your shell's startup file as its header says. zsh is tested with stand-ins for `compadd` and `_files`, as its completion system needs a terminal.
