---
category: Added
changelog: rusty-mux-ctl terminfo check|setup|remove installs rusty_term's terminfo entry in ~/.terminfo on an SSH host (C47).
---
## 2026-11-10 - rusty-mux-ctl remote terminfo

- **Added:** `rusty-mux-ctl terminfo check DEST` reports whether rusty_term's terminfo entry is in `~/.terminfo` on an SSH host, and whether `tic` is there. `terminfo setup DEST` compiles it into the remote `~/.terminfo`, and `terminfo remove DEST` deletes our two entries. Both print the ssh command first and take `--dry-run`. `$RUSTY_MUX_SSH` names another ssh.
- **Known limitations:** the remote needs `tic` and an `xterm-256color` entry. Mux blocks set no `TERM`, so the entry helps programs reached by a plain `ssh` from a rusty_term tab.
