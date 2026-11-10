---
category: Added
changelog: rusty-mux-ctl ssh setup --write adds a host to its own ssh config file with one Include line, and ssh remove --write takes it out again (C48).
---
## 2026-11-10 - rusty-mux-ctl ssh setup --write and ssh remove

- **Added:** `rusty-mux-ctl ssh setup DEST --write` writes the Host block to `~/.ssh/rusty_mux/config` and adds one marked `Include` line at the top of `~/.ssh/config`, keeping `~/.ssh/config.rusty-mux.bak` before the first edit. Each write is checked with `ssh -G -F` first and a symlinked config is refused. Only a plain alias is written, and never one your own config already has. `ssh remove DEST --write` undoes it; without `--write` both commands only print.
- **Fixed:** `ssh check` no longer reports an unknown host key when `StrictHostKeyChecking` is off or `accept-new`, reads `GlobalKnownHostsFile`, and finds hosts behind nested Includes and `Key=value` lines.
- **Known limitations:** Includes with wildcards are still not followed when looking for an existing host.
