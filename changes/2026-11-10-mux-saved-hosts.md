---
category: Added
changelog: rusty-mux-ctl saves hosts and reaches one with --host NAME, trying its addresses in order, with a hosts-changed event for followers (C35).
---
## 2026-11-10 - rusty-mux-ctl saved hosts

- **Added:** `rusty-mux-ctl hosts list|add|remove` keep named servers in `rusty_mux/hosts.json` in the config directory, each with addresses tried in order. `--host NAME` (or `--host=NAME`) sends a call to the first address that connects; with every address down it exits 4 and names each one's reason. `events --follow` and `watch` print `hosts-changed` when the saved list changes, and an `on` rule can run on it.
- **Known limitations:** `ssh` addresses are saved but not reached yet; `--host` skips them as unreachable. Only `rusty-mux-ctl` takes `--host`. On Windows the file needs `XDG_CONFIG_HOME` or `HOME`, as the config file does.
