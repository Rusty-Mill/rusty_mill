---
category: Added
changelog: rusty-mux-ctl reads hosts declared in config.json; a declared host wins over a saved one of the same name.
---
## 2026-11-11 - Hosts declared in the mux config

- **Added:** `config.json` takes `hosts` (the same host objects as `hosts.json`) and `home` (a host name). `rusty-mux-ctl --host NAME` uses a declared host before a saved one, and reads only those two keys, so a bad action or rule never stops `--host`.
- **Added:** `hosts list` marks declared hosts `(config)`, the home `(home)` and a saved host a declared one hides `(saved, hidden by config)`; `--json` adds `source`, `home` and `shadowed`. `hosts add` refuses a declared name, and `hosts remove` refuses a name only the config has. `events --follow` and `watch` print `hosts-changed` when the declared hosts or the home change. `config check` refuses a home that names no host.
- **Known limitations:** `home` is only declared and checked so far; what it does is still to be decided. Hosts are not synced between machines.
